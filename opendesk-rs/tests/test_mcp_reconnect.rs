//! The MCP server must recover when a cached peer connection drops, without
//! running a non-idempotent request twice.

use anyhow::Result;
use opendesk_rs::mcp::server::McpServer;
use opendesk_rs::protocol::identity::Identity;
use opendesk_rs::protocol::storage::TrustedPeers;
use opendesk_rs::remote::server::OpendeskServer;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// TCP proxy in front of the opendesk server whose live connections can be
/// killed on demand, either abruptly (RST) or gracefully (FIN).
struct DropProxy {
    port: u16,
    kill: broadcast::Sender<bool>,
}

impl DropProxy {
    async fn start(upstream: u16) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let (kill, _) = broadcast::channel(4);
        let kill_tx = kill.clone();
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let mut kill_rx = kill_tx.subscribe();
                tokio::spawn(async move {
                    let Ok(mut server) = TcpStream::connect(("127.0.0.1", upstream)).await else {
                        return;
                    };
                    let rst = tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut client, &mut server) => false,
                        r = kill_rx.recv() => r.unwrap_or(false),
                    };
                    if rst {
                        #[allow(deprecated)]
                        let _ = client.set_linger(Some(Duration::ZERO));
                    }
                });
            }
        });
        Ok(Self { port, kill })
    }

    /// Kill every open connection, then give the client OS time to see it.
    async fn drop_all(&self, rst: bool) {
        let _ = self.kill.send(rst);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn exec_args(marker: &Path) -> Value {
    let p = marker.display();
    #[cfg(target_os = "windows")]
    let (command, shell) = (
        format!("Add-Content -Path '{p}' -Value run; Write-Output RECONNECT_OK"),
        "powershell",
    );
    #[cfg(not(target_os = "windows"))]
    let (command, shell) = (format!("echo run >> '{p}'; echo RECONNECT_OK"), "sh");
    json!({ "peer": "my-server", "command": command, "shell": shell, "timeout_secs": 20 })
}

fn runs(marker: &Path) -> usize {
    std::fs::read_to_string(marker)
        .map(|s| s.lines().filter(|l| l.trim() == "run").count())
        .unwrap_or(0)
}

fn text(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn test_mcp_reconnects_after_dropped_connection() -> Result<()> {
    let server_dir = tempfile::tempdir()?;
    let client_dir = tempfile::tempdir()?;
    let marker = client_dir.path().join("runs.txt");
    let port = 18431;

    let server_id = Identity::load_or_create(Some(server_dir.path()))?;
    let client_id = Identity::load_or_create(Some(client_dir.path()))?;
    TrustedPeers::new(Some(server_dir.path())).add(&client_id.public_bytes(), "my-client", "")?;

    let server = Arc::new(OpendeskServer::new(
        "127.0.0.1",
        port,
        Some(server_dir.path()),
        vec![],
        None,
    )?);
    let s_clone = server.clone();
    let server_task = tokio::spawn(async move { s_clone.serve_forever().await });
    tokio::time::sleep(Duration::from_millis(150)).await;

    // The client reaches the server only through the proxy.
    let proxy = DropProxy::start(port).await?;
    let client_trusted = TrustedPeers::new(Some(client_dir.path()));
    client_trusted.add(&server_id.public_bytes(), "my-server", "")?;
    client_trusted.cache_endpoint(&server_id.public_bytes(), "127.0.0.1", proxy.port)?;

    let mut mcp = McpServer::with_config(Some(client_dir.path().to_path_buf()), None, None);
    let args = exec_args(&marker);

    let out = text(&mcp.call_tool("opendesk_exec", &args).await?);
    assert!(out.contains("RECONNECT_OK"), "{out}");
    assert_eq!(runs(&marker), 1);

    // Connection reset: the next send fails before anything reaches the
    // peer, so the call is transparently retried on a fresh connection.
    proxy.drop_all(true).await;
    let out = text(&mcp.call_tool("opendesk_exec", &args).await?);
    assert!(out.contains("RECONNECT_OK"), "{out}");
    assert_eq!(runs(&marker), 2, "request must run exactly once");
    let status = text(&mcp.call_tool("opendesk_status", &json!({})).await?);
    assert!(status.contains("Open connections: my-server"), "{status}");

    // Graceful close: the send succeeds locally and only the response read
    // fails, so the request may have been delivered and is not retried...
    proxy.drop_all(false).await;
    let err = mcp
        .call_tool("opendesk_exec", &args)
        .await
        .expect_err("lost response must not be silently retried");
    assert!(err.to_string().contains("not retried"), "{err}");
    assert_eq!(runs(&marker), 2);
    // ...but the dead connection is gone, and the next call reconnects.
    let out = text(&mcp.call_tool("opendesk_exec", &args).await?);
    assert!(out.contains("RECONNECT_OK"), "{out}");
    assert_eq!(runs(&marker), 3);

    // With an idle liveness check, a gracefully closed connection is detected
    // by the ping before reuse, and the call succeeds on a fresh connection.
    mcp.set_idle_ping_after(Duration::ZERO);
    proxy.drop_all(false).await;
    let out = text(&mcp.call_tool("opendesk_exec", &args).await?);
    assert!(out.contains("RECONNECT_OK"), "{out}");
    assert_eq!(runs(&marker), 4);

    server_task.abort();
    Ok(())
}
