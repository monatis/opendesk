use anyhow::Result;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use opendesk_rs::app::{AppState, create_router};
use opendesk_rs::automation::scheduler::ScheduleStore;
use opendesk_rs::computer::permissions::check_all;
use opendesk_rs::remote::admin::{ActiveSessionEntry, AdminClient, AdminServer, SessionRegistry};
use opendesk_rs::remote::audit::{AuditLog, format_audit_entry, summarise};
use opendesk_rs::remote::server::OpendeskServer;

#[tokio::test]
async fn test_audit_log_record_and_query() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let audit = AuditLog::new(Some(tmp.path()));

    let pk = [42u8; 32];
    audit
        .record_session_opened(&pk, "macbook", "sess1", "192.168.1.50:8423", "direct")
        .await;

    audit
        .record_call(
            &pk,
            "macbook",
            "sess1",
            "display.capture",
            &json!({}),
            "ok",
            None,
            None,
        )
        .await;

    audit
        .record_call(
            &pk,
            "macbook",
            "sess1",
            "input.text",
            &json!({ "text_input": { "text": "hello from test" } }),
            "ok",
            None,
            None,
        )
        .await;

    audit
        .record_session_closed(&pk, "macbook", "sess1", 12.345, "normal")
        .await;

    let entries = audit.iter_entries(None);
    assert_eq!(entries.len(), 4);

    assert_eq!(entries[0]["type"], "session.opened");
    assert_eq!(entries[0]["peer"]["name"], "macbook");
    assert_eq!(entries[0]["session_id"], "sess1");

    assert_eq!(entries[1]["type"], "call");
    assert_eq!(entries[1]["method"], "display.capture");

    assert_eq!(entries[2]["type"], "call");
    assert_eq!(entries[2]["method"], "input.text");
    assert!(
        entries[2]["summary"]
            .as_str()
            .unwrap()
            .contains("hello from test")
    );

    assert_eq!(entries[3]["type"], "session.closed");

    // Test console formatting doesn't panic
    for e in &entries {
        let formatted = format_audit_entry(e);
        assert!(!formatted.is_empty());
    }

    Ok(())
}

#[tokio::test]
async fn test_summarise_parity() {
    let s1 = summarise(
        "input.pointer",
        &json!({
            "event": { "action": "move", "point": { "x": 100, "y": 200 } }
        }),
    );
    assert_eq!(s1, "send pointer move at (100, 200)");

    let s2 = summarise(
        "input.text",
        &json!({
            "text_input": { "text": "hello" }
        }),
    );
    assert_eq!(s2, "type 5 chars: 'hello'");

    let s3 = summarise(
        "process.shell",
        &json!({
            "command": "echo test"
        }),
    );
    assert_eq!(s3, "run shell: 'echo test'");
}

#[tokio::test]
async fn test_admin_ipc_session_list_and_eviction() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let registry = SessionRegistry::new();

    let (tx1, mut rx1) = tokio::sync::mpsc::channel(1);
    let (tx2, mut rx2) = tokio::sync::mpsc::channel(1);

    registry
        .add(ActiveSessionEntry {
            id: "sess_a".into(),
            peer_name: "laptop".into(),
            peer_public: [1u8; 32],
            remote_addr: "10.0.0.1:1234".into(),
            started_at: 1000.0,
            mode: "direct".into(),
            evict_tx: tx1,
        })
        .await;

    registry
        .add(ActiveSessionEntry {
            id: "sess_b".into(),
            peer_name: "desktop".into(),
            peer_public: [2u8; 32],
            remote_addr: "10.0.0.2:5678".into(),
            started_at: 2000.0,
            mode: "direct".into(),
            evict_tx: tx2,
        })
        .await;

    let mut admin_server = AdminServer::new(registry.clone(), Some(tmp.path()));
    admin_server.start().await?;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = AdminClient::connect(Some(tmp.path())).await?;
    let list = client.list_sessions().await?;
    assert_eq!(list.len(), 2);

    // Test killing single session
    let killed = client.kill("sess_a").await?;
    assert!(killed);
    assert_eq!(rx1.recv().await, Some("admin_disconnect".to_string()));

    // Test list after kill
    let mut client2 = AdminClient::connect(Some(tmp.path())).await?;
    let list2 = client2.list_sessions().await?;
    assert_eq!(list2.len(), 1);
    assert_eq!(list2[0].id, "sess_b");

    // Test kill_all
    let mut client3 = AdminClient::connect(Some(tmp.path())).await?;
    let count = client3.kill_all().await?;
    assert_eq!(count, 1);
    assert_eq!(rx2.recv().await, Some("admin_disconnect".to_string()));

    admin_server.stop();
    Ok(())
}

#[tokio::test]
async fn test_scheduler_store_crud() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let store = ScheduleStore::new(tmp.path());

    assert!(store.all().is_empty());

    let e1 = store.add("daily-backup", "backup databases", "every 24h")?;
    assert_eq!(e1.name, "daily-backup");
    assert_eq!(e1.task, "backup databases");

    let entries = store.all();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "daily-backup");

    let removed = store.remove("daily-backup")?;
    assert!(removed);
    assert!(store.all().is_empty());

    Ok(())
}

#[tokio::test]
async fn test_permissions_check_all() {
    let statuses = check_all();
    // On Windows/Linux should be empty, on macOS should contain Accessibility & Screen Recording
    if cfg!(target_os = "macos") {
        assert_eq!(statuses.len(), 2);
    } else {
        assert!(statuses.is_empty());
    }
}

#[tokio::test]
async fn test_app_web_endpoints() -> Result<()> {
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let tmp = tempfile::tempdir()?;
    let server = Arc::new(OpendeskServer::new(
        "127.0.0.1",
        19999,
        Some(tmp.path()),
        vec![],
        None,
    )?);

    let state = AppState {
        home: Some(tmp.path().to_path_buf()),
        server,
        outbound: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        pairing_code: Arc::new(tokio::sync::Mutex::new(None)),
        pairing_result: Arc::new(tokio::sync::Mutex::new(None)),
        pairing_abort_tx: Arc::new(tokio::sync::Mutex::new(None)),
        rendezvous: Arc::new(tokio::sync::Mutex::new(
            opendesk_rs::protocol::storage::GlobalRendezvousConfig::default(),
        )),
    };

    let router = create_router(state);

    // 1. Test GET / serves index.html
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(res.status(), 200);

    // 2. Test GET /static/styles.css
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/static/styles.css")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(res.status(), 200);

    // 3. Test GET /api/state
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/state")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(res.status(), 200);

    let body = res.into_body().collect().await?.to_bytes();
    let val: serde_json::Value = serde_json::from_slice(&body)?;
    assert!(val.get("identity").is_some());
    assert!(val.get("trusted_peers").is_some());

    // 4. Test that /api/peer/local/privacy is rejected with 400 Bad Request (safety restriction)
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/peer/local/privacy")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(res.status(), 400);

    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/peer/local/privacy")
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(r#"{"lock_input":true}"#))?,
        )
        .await?;
    assert_eq!(res.status(), 400);

    Ok(())
}
