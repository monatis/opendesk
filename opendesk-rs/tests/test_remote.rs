use anyhow::Result;
use opendesk_rs::protocol::identity::Identity;
use opendesk_rs::protocol::storage::TrustedPeers;
use opendesk_rs::remote::client::{connect as remote_connect, pair_with};
use opendesk_rs::remote::rendezvous::RendezvousServer;
use opendesk_rs::remote::server::OpendeskServer;
use std::sync::Arc;

/// Tests that round-trip the real system clipboard run one at a time;
/// in parallel they overwrite each other's text.
static CLIPBOARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn test_live_pairing_and_rpc() -> Result<()> {
    let _clipboard = CLIPBOARD.lock().await;
    let server_dir = tempfile::tempdir()?;
    let client_dir = tempfile::tempdir()?;

    let port = 18423;
    let code = "987654";

    let server = OpendeskServer::new("127.0.0.1", port, Some(server_dir.path()), vec![], None)?;

    // Spawn server pair listener
    let server_task = tokio::spawn(async move { server.run_pair(code, 10).await });

    // Small delay to ensure server socket bound
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Client pairs with server
    let (remote, server_pub) = pair_with(
        Some("127.0.0.1"),
        Some(port),
        code,
        Some("my-server"),
        None,
        None,
        None,
        Some(client_dir.path()),
    )
    .await?;

    let pair_res = server_task.await??;
    assert_eq!(pair_res, {
        let client_id = Identity::load_or_create(Some(client_dir.path()))?;
        client_id.public_bytes()
    });

    let s_id = Identity::load_or_create(Some(server_dir.path()))?;
    assert_eq!(server_pub, s_id.public_bytes());

    // Test calling an RPC method on the newly paired remote computer
    remote.clipboard_write("PAIRING_TEST_OK").await?;
    let text = remote.clipboard_read().await?;
    assert_eq!(text, "PAIRING_TEST_OK");

    Ok(())
}

#[tokio::test]
async fn test_live_reconnect_and_serve() -> Result<()> {
    let _clipboard = CLIPBOARD.lock().await;
    let server_dir = tempfile::tempdir()?;
    let client_dir = tempfile::tempdir()?;

    let port = 18425;

    let server_id = Identity::load_or_create(Some(server_dir.path()))?;
    let client_id = Identity::load_or_create(Some(client_dir.path()))?;

    // Pre-populate trusted peers on both sides
    let server_trusted = TrustedPeers::new(Some(server_dir.path()));
    server_trusted.add(&client_id.public_bytes(), "my-client", "")?;

    let client_trusted = TrustedPeers::new(Some(client_dir.path()));
    client_trusted.add(&server_id.public_bytes(), "my-server", "")?;
    client_trusted.cache_endpoint(&server_id.public_bytes(), "127.0.0.1", port)?;

    let server = Arc::new(OpendeskServer::new(
        "127.0.0.1",
        port,
        Some(server_dir.path()),
        vec![],
        None,
    )?);

    let s_clone = server.clone();
    let server_task = tokio::spawn(async move { s_clone.serve_forever().await });

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Connect via authenticated static keys
    let remote = remote_connect(Some("my-server"), None, None, Some(client_dir.path())).await?;

    // Test remote clipboard write and read
    remote
        .clipboard_write("OPENDESK_RUST_REMOTE_SUCCESS")
        .await?;
    let read_back = remote.clipboard_read().await?;
    assert_eq!(read_back, "OPENDESK_RUST_REMOTE_SUCCESS");

    // Test privacy RPC capabilities and set/get
    assert!(remote.capabilities().contains_key("system.privacy"));
    assert!(remote.capabilities().contains_key("process.exec"));
    assert!(remote.capabilities().contains_key("input.sequence"));

    #[cfg(target_os = "windows")]
    let exec_res = remote
        .exec(
            "Write-Output 'OPENDESK_REMOTE_EXEC_SUCCESS'",
            Some("powershell"),
            Some(10),
        )
        .await?;
    #[cfg(not(target_os = "windows"))]
    let exec_res = remote
        .exec("echo OPENDESK_REMOTE_EXEC_SUCCESS", Some("sh"), Some(10))
        .await?;

    assert!(exec_res.success);
    assert!(exec_res.stdout.contains("OPENDESK_REMOTE_EXEC_SUCCESS"));
    assert_eq!(exec_res.exit_code, 0);

    // Test remote sequence RPC
    let seq_steps = vec![opendesk_rs::computer::local::KeyboardStep {
        delay_ms: Some(20),
        ..Default::default()
    }];
    remote.keyboard_sequence(&seq_steps).await?;

    let st = remote.set_privacy(true, false).await?;
    assert!(st.lock_input);
    let st_status = remote.get_privacy().await?;
    assert!(st_status.lock_input);
    let st_reset = remote.set_privacy(false, false).await?;
    assert!(!st_reset.lock_input);

    // Abort server background loop
    server_task.abort();
    Ok(())
}

#[tokio::test]
async fn test_live_exec_timeout_returns_partial_output() -> Result<()> {
    let server_dir = tempfile::tempdir()?;
    let client_dir = tempfile::tempdir()?;
    let port = 18432;

    let server_id = Identity::load_or_create(Some(server_dir.path()))?;
    let client_id = Identity::load_or_create(Some(client_dir.path()))?;
    TrustedPeers::new(Some(server_dir.path())).add(&client_id.public_bytes(), "my-client", "")?;
    let client_trusted = TrustedPeers::new(Some(client_dir.path()));
    client_trusted.add(&server_id.public_bytes(), "my-server", "")?;
    client_trusted.cache_endpoint(&server_id.public_bytes(), "127.0.0.1", port)?;

    let server = Arc::new(OpendeskServer::new(
        "127.0.0.1",
        port,
        Some(server_dir.path()),
        vec![],
        None,
    )?);
    let s_clone = server.clone();
    let server_task = tokio::spawn(async move { s_clone.serve_forever().await });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let remote = remote_connect(Some("my-server"), None, None, Some(client_dir.path())).await?;

    #[cfg(target_os = "windows")]
    let (cmd, shell) = (
        "Write-Output \"partial \"\"quoted\"\"\"; Start-Sleep 120",
        "powershell",
    );
    #[cfg(not(target_os = "windows"))]
    let (cmd, shell) = ("echo 'partial \"quoted\"'; sleep 120", "sh");

    // Generous enough for PowerShell to start and print under parallel test
    // load, far below the 120 s the command would otherwise run.
    let start = std::time::Instant::now();
    let res = remote.exec(cmd, Some(shell), Some(10)).await?;
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    assert!(res.timed_out, "{res:?}");
    assert!(!res.success);
    assert!(res.stdout.contains("partial \"quoted\""), "{res:?}");

    // The session is still usable after a timed-out command.
    let res = remote
        .exec("echo still-alive", Some(shell), Some(20))
        .await?;
    assert!(res.stdout.contains("still-alive"), "{res:?}");
    assert!(!res.timed_out);

    server_task.abort();
    Ok(())
}

#[tokio::test]
async fn test_live_rendezvous_signaling_and_relay() -> Result<()> {
    let _clipboard = CLIPBOARD.lock().await;
    let r_port = 18426;

    let r_server = RendezvousServer::new("127.0.0.1", r_port, Some("test-token".to_string()));
    let r_task = tokio::spawn(async move { r_server.serve_forever().await });

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let server_dir = tempfile::tempdir()?;
    let client_dir = tempfile::tempdir()?;

    let server_id = Identity::load_or_create(Some(server_dir.path()))?;
    let client_id = Identity::load_or_create(Some(client_dir.path()))?;

    // Pre-trust each other
    let server_trusted = TrustedPeers::new(Some(server_dir.path()));
    server_trusted.add(&client_id.public_bytes(), "my-client", "")?;

    let client_trusted = TrustedPeers::new(Some(client_dir.path()));
    client_trusted.add(&server_id.public_bytes(), "my-server", "")?;

    let r_url = format!("ws://127.0.0.1:{}", r_port);

    // Controlled machine starts server with outbound rendezvous connection
    let server = Arc::new(OpendeskServer::new(
        "127.0.0.1",
        18427, // Local port not even needed for incoming
        Some(server_dir.path()),
        vec![r_url.clone()],
        Some("test-token".to_string()),
    )?);

    let s_clone = server.clone();
    let server_task = tokio::spawn(async move { s_clone.serve_forever().await });

    // Wait for agent to connect and register on rendezvous
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // Controller connects through rendezvous relay
    let remote = remote_connect(
        Some("my-server"),
        Some(&r_url),
        Some("test-token"),
        Some(client_dir.path()),
    )
    .await?;

    // Execute command through the relay!
    remote.clipboard_write("RENDEZVOUS_RELAY_OK").await?;
    let text = remote.clipboard_read().await?;
    assert_eq!(text, "RENDEZVOUS_RELAY_OK");

    server_task.abort();
    r_task.abort();
    Ok(())
}

/// Polls `f` until it returns `expected` or `timeout` elapses; returns the last value seen.
async fn wait_for_count<F: Fn() -> usize>(
    f: F,
    expected: usize,
    timeout: std::time::Duration,
) -> usize {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let n = f();
        if n == expected || tokio::time::Instant::now() >= deadline {
            return n;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn test_rendezvous_relay_sessions_do_not_leak_connections() -> Result<()> {
    let _clipboard = CLIPBOARD.lock().await;
    let r_port = 18428;

    let r_server = Arc::new(RendezvousServer::new(
        "127.0.0.1",
        r_port,
        Some("test-token".to_string()),
    ));
    let r_clone = r_server.clone();
    let r_task = tokio::spawn(async move { r_clone.serve_forever().await });

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let server_dir = tempfile::tempdir()?;
    let client_dir = tempfile::tempdir()?;

    let server_id = Identity::load_or_create(Some(server_dir.path()))?;
    let client_id = Identity::load_or_create(Some(client_dir.path()))?;

    let server_trusted = TrustedPeers::new(Some(server_dir.path()));
    server_trusted.add(&client_id.public_bytes(), "my-client", "")?;

    let client_trusted = TrustedPeers::new(Some(client_dir.path()));
    client_trusted.add(&server_id.public_bytes(), "my-server", "")?;

    let r_url = format!("ws://127.0.0.1:{}", r_port);

    let server = Arc::new(OpendeskServer::new(
        "127.0.0.1",
        18429,
        Some(server_dir.path()),
        vec![r_url.clone()],
        Some("test-token".to_string()),
    )?);
    let s_clone = server.clone();
    let server_task = tokio::spawn(async move { s_clone.serve_forever().await });

    // Only the agent's long-lived control connection should be open.
    let timeout = std::time::Duration::from_secs(5);
    let baseline = wait_for_count(|| r_server.active_connections(), 1, timeout).await;
    assert_eq!(baseline, 1, "agent control connection did not register");

    for i in 0..5 {
        let remote = remote_connect(
            Some("my-server"),
            Some(&r_url),
            Some("test-token"),
            Some(client_dir.path()),
        )
        .await?;

        // Controller leg + agent leg + control connection.
        let during = wait_for_count(|| r_server.active_connections(), 3, timeout).await;
        assert_eq!(
            during, 3,
            "unexpected relay connection count in session {}",
            i
        );

        let marker = format!("RELAY_LEAK_TEST_{}", i);
        remote.clipboard_write(&marker).await?;
        assert_eq!(remote.clipboard_read().await?, marker);

        // Controller disconnects: both relay legs must be torn down.
        drop(remote);
        let after = wait_for_count(|| r_server.active_connections(), 1, timeout).await;
        assert_eq!(
            after, 1,
            "relay connections leaked after closing session {}",
            i
        );
        assert_eq!(r_server.active_sessions().await, 0);
    }

    server_task.abort();
    r_task.abort();
    Ok(())
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn test_live_input_guard_is_enforced_by_the_peer() -> Result<()> {
    let server_dir = tempfile::tempdir()?;
    let client_dir = tempfile::tempdir()?;
    let port = 18433;

    let server_id = Identity::load_or_create(Some(server_dir.path()))?;
    let client_id = Identity::load_or_create(Some(client_dir.path()))?;
    TrustedPeers::new(Some(server_dir.path())).add(&client_id.public_bytes(), "my-client", "")?;
    let client_trusted = TrustedPeers::new(Some(client_dir.path()));
    client_trusted.add(&server_id.public_bytes(), "my-server", "")?;
    client_trusted.cache_endpoint(&server_id.public_bytes(), "127.0.0.1", port)?;

    let server = Arc::new(OpendeskServer::new(
        "127.0.0.1",
        port,
        Some(server_dir.path()),
        vec![],
        None,
    )?);
    let s_clone = server.clone();
    let server_task = tokio::spawn(async move { s_clone.serve_forever().await });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let remote = remote_connect(Some("my-server"), None, None, Some(client_dir.path())).await?;
    assert!(remote.capabilities().contains_key("input.guard"));

    // The peer refuses before sending anything, so no key reaches the desktop.
    remote.set_input_guard(Some(opendesk_rs::computer::foreground::WindowGuard {
        title: Some("opendesk-no-such-window-7f3a".into()),
        process: None,
    }));
    let err = remote
        .keyboard_press("shift")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("input refused"), "{err}");
    let err = remote.mouse_move(0, 0).await.unwrap_err().to_string();
    assert!(err.contains("input refused"), "{err}");

    // Non-input RPCs are unaffected, and the session stays usable.
    let res = remote.exec("echo guard-ok", Some("cmd"), Some(20)).await?;
    assert!(res.stdout.contains("guard-ok"), "{res:?}");

    server_task.abort();
    Ok(())
}
