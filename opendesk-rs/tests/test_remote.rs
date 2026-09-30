use anyhow::Result;
use opendesk_rs::protocol::identity::Identity;
use opendesk_rs::protocol::storage::TrustedPeers;
use opendesk_rs::remote::client::{connect as remote_connect, pair_with};
use opendesk_rs::remote::rendezvous::RendezvousServer;
use opendesk_rs::remote::server::OpendeskServer;
use std::sync::Arc;

#[tokio::test]
async fn test_live_pairing_and_rpc() -> Result<()> {
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
async fn test_live_rendezvous_signaling_and_relay() -> Result<()> {
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
