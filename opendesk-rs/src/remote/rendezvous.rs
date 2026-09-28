//! Internet Remote Transport — Rendezvous & Relay service, Agent, and Client.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::computer::local::LocalComputer;
use crate::protocol::identity::Identity;
use crate::protocol::storage::{fingerprint, read_description, TrustedPeers};
use super::server::{run_session_loop, ActiveSession};
use super::transport::WebSocketTransport;

pub const DEFAULT_RENDEZVOUS_PORT: u16 = 8424;

// ---------------------------------------------------------------------------
// Rendezvous Server
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct RegisteredAgent {
    public_key: String,
    name: String,
    description: String,
    remote_addr: String,
    control_tx: mpsc::Sender<String>,
}

#[allow(dead_code)]
struct PendingSession {
    session_id: String,
    target: String,
    agent_tx: Option<mpsc::Sender<Message>>,
    controller_tx: Option<mpsc::Sender<Message>>,
}

pub struct RendezvousServer {
    host: String,
    port: u16,
    token: Option<String>,
    agents: Arc<Mutex<HashMap<String, RegisteredAgent>>>,
    sessions: Arc<Mutex<HashMap<String, PendingSession>>>,
}

impl RendezvousServer {
    pub fn new(host: &str, port: u16, token: Option<String>) -> Self {
        Self {
            host: host.to_string(),
            port,
            token,
            agents: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn serve_forever(&self) -> Result<()> {
        let bind_addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("failed to bind rendezvous to {}", bind_addr))?;

        info!("opendesk rendezvous listening on {}", bind_addr);
        println!("opendesk rendezvous listening on {}", bind_addr);
        if self.token.is_some() {
            println!("  Token authentication required");
        }

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            let token = self.token.clone();
            let agents = self.agents.clone();
            let sessions = self.sessions.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_rendezvous_ws(stream, remote_addr, token, agents, sessions).await {
                    warn!("Rendezvous connection error from {}: {}", remote_addr, e);
                }
            });
        }
    }
}

async fn handle_rendezvous_ws(
    stream: TcpStream,
    remote_addr: SocketAddr,
    auth_token: Option<String>,
    agents: Arc<Mutex<HashMap<String, RegisteredAgent>>>,
    sessions: Arc<Mutex<HashMap<String, PendingSession>>>,
) -> Result<()> {
    let mut ws = tokio_tungstenite::accept_async(stream).await?;

    // Read first message (JSON signaling)
    let first_msg = match ws.next().await {
        Some(Ok(Message::Text(t))) => t,
        _ => {
            let _ = ws.close(None).await;
            return Ok(());
        }
    };

    let data: Value = serde_json::from_str(&first_msg)
        .context("invalid JSON in rendezvous first message")?;

    // Token check
    if let Some(expected_token) = &auth_token {
        let given_token = data.get("token").and_then(|v| v.as_str());
        if given_token != Some(expected_token.as_str()) {
            let _ = ws.send(Message::Text(json!({ "status": "error", "error": "unauthorized" }).to_string().into())).await;
            let _ = ws.close(None).await;
            return Ok(());
        }
    }

    let action = data.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "register" => {
            let public_key = data.get("public_key").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing public_key"))?;
            let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let description = data.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();

            let (tx, mut rx) = mpsc::channel::<String>(32);
            {
                let mut reg = agents.lock().await;
                reg.insert(public_key.to_string(), RegisteredAgent {
                    public_key: public_key.to_string(),
                    name: name.clone(),
                    description: description.clone(),
                    remote_addr: remote_addr.to_string(),
                    control_tx: tx,
                });
            }

            info!("Registered agent {} ({}) from {}", name, &public_key[..8.min(public_key.len())], remote_addr);

            ws.send(Message::Text(json!({
                "status": "ok",
                "action": "registered",
                "public_key": public_key,
                "client_ip": remote_addr.ip().to_string(),
                "client_port": remote_addr.port(),
            }).to_string().into())).await?;

            // Agent loop: forward outbound messages and answer ping
            let pk_str = public_key.to_string();
            let agents_cleanup = agents.clone();

            loop {
                tokio::select! {
                    Some(to_agent) = rx.recv() => {
                        if let Err(_) = ws.send(Message::Text(to_agent.into())).await {
                            break;
                        }
                    }
                    msg = ws.next() => {
                        match msg {
                            Some(Ok(Message::Text(t))) => {
                                if let Ok(val) = serde_json::from_str::<Value>(&t)
                                    && val.get("action").and_then(|v| v.as_str()) == Some("ping") {
                                        let _ = ws.send(Message::Text(json!({ "action": "pong" }).to_string().into())).await;
                                    }
                            }
                            Some(Ok(Message::Ping(p))) => {
                                let _ = ws.send(Message::Pong(p)).await;
                            }
                            _ => break,
                        }
                    }
                }
            }

            let mut reg = agents_cleanup.lock().await;
            reg.remove(&pk_str);
            info!("Agent {} disconnected", pk_str);
        }
        "connect" => {
            let target = data.get("target").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing target"))?;
            let session_id = data.get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{:08x}", rand::random::<u32>()));

            let agent_control = {
                let reg = agents.lock().await;
                reg.get(target).map(|a| a.control_tx.clone())
            };

            let control_tx = match agent_control {
                Some(tx) => tx,
                None => {
                    let _ = ws.send(Message::Text(json!({
                        "status": "error",
                        "error": "target_offline",
                        "message": "Target peer is not registered or offline",
                    }).to_string().into())).await;
                    return Ok(());
                }
            };

            // Register session
            let (ctrl_out_tx, mut ctrl_out_rx) = mpsc::channel::<Message>(64);
            {
                let mut sess_map = sessions.lock().await;
                sess_map.insert(session_id.clone(), PendingSession {
                    session_id: session_id.clone(),
                    target: target.to_string(),
                    agent_tx: None,
                    controller_tx: Some(ctrl_out_tx),
                });
            }

            // Signal agent
            let _ = control_tx.send(json!({
                "action": "session_request",
                "session_id": session_id,
            }).to_string()).await;

            // Wait for agent to join or loop
            let mut agent_channel: Option<mpsc::Sender<Message>> = None;

            loop {
                tokio::select! {
                    Some(out_msg) = ctrl_out_rx.recv() => {
                        if let Err(_) = ws.send(out_msg).await {
                            break;
                        }
                    }
                    in_msg = ws.next() => {
                        match in_msg {
                            Some(Ok(Message::Text(t))) => {
                                if let Ok(val) = serde_json::from_str::<Value>(&t) {
                                    let act = val.get("action").and_then(|v| v.as_str());
                                    if act == Some("ready") {
                                        let _ = ws.send(Message::Text(json!({
                                            "status": "relay_active",
                                            "session_id": session_id,
                                        }).to_string().into())).await;

                                        // Lookup agent channel
                                        let sess_map = sessions.lock().await;
                                        if let Some(s) = sess_map.get(&session_id) {
                                            agent_channel = s.agent_tx.clone();
                                        }
                                    }
                                }
                            }
                            Some(Ok(Message::Binary(bin))) => {
                                if agent_channel.is_none() {
                                    let sess_map = sessions.lock().await;
                                    if let Some(s) = sess_map.get(&session_id) {
                                        agent_channel = s.agent_tx.clone();
                                    }
                                }
                                if let Some(a_tx) = &agent_channel {
                                    let _ = a_tx.send(Message::Binary(bin)).await;
                                }
                            }
                            _ => break,
                        }
                    }
                }
            }

            let mut sess_map = sessions.lock().await;
            sess_map.remove(&session_id);
        }
        "join" => {
            let session_id = data.get("session_id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing session_id"))?;
            let (agent_out_tx, mut agent_out_rx) = mpsc::channel::<Message>(64);
            let ctrl_channel = {
                let mut sess_map = sessions.lock().await;
                if let Some(s) = sess_map.get_mut(session_id) {
                    s.agent_tx = Some(agent_out_tx);
                    s.controller_tx.clone()
                } else {
                    let _ = ws.send(Message::Text(json!({ "status": "error", "error": "session_not_found" }).to_string().into())).await;
                    return Ok(());
                }
            };

            // Notify controller
            if let Some(c_tx) = &ctrl_channel {
                let _ = c_tx.send(Message::Text(json!({
                    "status": "joined",
                    "session_id": session_id,
                }).to_string().into())).await;
            }

            let _ = ws.send(Message::Text(json!({
                "status": "joined",
                "session_id": session_id,
            }).to_string().into())).await;

            loop {
                tokio::select! {
                    Some(out_msg) = agent_out_rx.recv() => {
                        if let Err(_) = ws.send(out_msg).await {
                            break;
                        }
                    }
                    in_msg = ws.next() => {
                        match in_msg {
                            Some(Ok(Message::Text(t))) => {
                                if let Ok(val) = serde_json::from_str::<Value>(&t) {
                                    let act = val.get("action").and_then(|v| v.as_str());
                                    if act == Some("ready") {
                                        let _ = ws.send(Message::Text(json!({
                                            "status": "relay_active",
                                            "session_id": session_id,
                                        }).to_string().into())).await;
                                    }
                                }
                            }
                            Some(Ok(Message::Binary(bin))) => {
                                if let Some(c_tx) = &ctrl_channel {
                                    let _ = c_tx.send(Message::Binary(bin)).await;
                                }
                            }
                            _ => break,
                        }
                    }
                }
            }
        }
        "list" => {
            let reg = agents.lock().await;
            let list: Vec<Value> = reg.values().map(|a| {
                let mut arr = [0u8; 32];
                let decoded = data_encoding::HEXLOWER.decode(a.public_key.as_bytes()).unwrap_or_default();
                if decoded.len() == 32 {
                    arr.copy_from_slice(&decoded);
                }
                json!({
                    "public_key": a.public_key,
                    "name": a.name,
                    "description": a.description,
                    "fingerprint": fingerprint(&arr),
                })
            }).collect();

            let _ = ws.send(Message::Text(json!({
                "status": "ok",
                "peers": list,
            }).to_string().into())).await;
        }
        _ => {
            let _ = ws.send(Message::Text(json!({ "status": "error", "error": "unknown_action" }).to_string().into())).await;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Rendezvous Agent (Controlled Machine Outbound)
// ---------------------------------------------------------------------------

pub async fn run_rendezvous_agent(
    rendezvous_url: &str,
    token: Option<&str>,
    identity: Identity,
    trusted: TrustedPeers,
    computer: Arc<LocalComputer>,
    active_session: Arc<Mutex<Option<ActiveSession>>>,
    home: Option<PathBuf>,
) -> Result<()> {
    loop {
        let (ws_stream, _) = match connect_async(rendezvous_url).await {
            Ok(res) => res,
            Err(e) => {
                warn!("Cannot connect to rendezvous {}: {}. Retrying in 5s...", rendezvous_url, e);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        info!("Connected to rendezvous: {}", rendezvous_url);
        let mut transport = WebSocketTransport::new_tls(ws_stream);

        let pk_hex = data_encoding::HEXLOWER.encode(&identity.public_bytes());
        let desc = read_description(home.as_deref());

        let mut reg_msg = json!({
            "action": "register",
            "public_key": pk_hex,
            "name": format!("peer-{}", &pk_hex[..6]),
            "description": desc,
        });
        if let Some(tok) = token {
            reg_msg["token"] = json!(tok);
        }

        if let Err(e) = transport.send_text(&reg_msg.to_string()).await {
            warn!("Failed to send register to rendezvous: {}", e);
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }

        // Read register response
        match transport.recv_text().await {
            Ok(resp) => {
                info!("Registered on rendezvous: {}", resp);
            }
            Err(e) => {
                warn!("Failed to receive register response: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        }

        // Keepalive / session request loop
        loop {
            tokio::select! {
                msg = transport.recv_text() => {
                    match msg {
                        Ok(text) => {
                            if let Ok(val) = serde_json::from_str::<Value>(&text)
                                && val.get("action").and_then(|v| v.as_str()) == Some("session_request")
                                    && let Some(session_id) = val.get("session_id").and_then(|v| v.as_str()) {
                                        info!("Incoming session request from rendezvous: {}", session_id);
                                        let r_url = rendezvous_url.to_string();
                                        let tok = token.map(|t| t.to_string());
                                        let sess_id = session_id.to_string();
                                        let id = identity.clone();
                                        let tr = trusted.clone();
                                        let comp = computer.clone();
                                        let act = active_session.clone();
                                        let hm = home.clone();

                                        tokio::spawn(async move {
                                            if let Err(e) = join_rendezvous_session(
                                                &r_url,
                                                tok.as_deref(),
                                                &sess_id,
                                                id,
                                                tr,
                                                comp,
                                                act,
                                                hm,
                                            ).await {
                                                warn!("Failed to handle rendezvous session {}: {}", sess_id, e);
                                            }
                                        });
                                    }
                        }
                        Err(_) => break,
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(25)) => {
                    let _ = transport.send_text(&json!({ "action": "ping" }).to_string()).await;
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

async fn join_rendezvous_session(
    rendezvous_url: &str,
    token: Option<&str>,
    session_id: &str,
    identity: Identity,
    trusted: TrustedPeers,
    computer: Arc<LocalComputer>,
    _active_session: Arc<Mutex<Option<ActiveSession>>>,
    home: Option<PathBuf>,
) -> Result<()> {
    let (ws_stream, _) = connect_async(rendezvous_url).await?;
    let mut transport = WebSocketTransport::new_tls(ws_stream);

    let mut join_msg = json!({
        "action": "join",
        "session_id": session_id,
    });
    if let Some(tok) = token {
        join_msg["token"] = json!(tok);
    }
    transport.send_text(&join_msg.to_string()).await?;

    let _ = transport.recv_text().await?;
    transport.send_text(&json!({ "action": "ready" }).to_string()).await?;
    let _ = transport.recv_text().await?;

    // Now transport is in binary relay mode!
    let session = crate::protocol::handshake::auth_server(&mut transport, &identity, &trusted).await?;
    let (_tx, mut rx) = tokio::sync::mpsc::channel(1);
    let audit = crate::remote::audit::AuditLog::new(home.as_deref());
    let peer_name = crate::protocol::storage::default_peer_name(&session.peer_public);
    run_session_loop(
        &mut transport,
        session.channel,
        computer,
        home.as_deref(),
        &mut rx,
        &session.peer_public,
        &peer_name,
        session_id,
        &audit,
        false,
    ).await
}

// ---------------------------------------------------------------------------
// Rendezvous Client (Controller Outbound)
// ---------------------------------------------------------------------------

pub async fn connect_via_rendezvous(
    rendezvous_url: &str,
    token: Option<&str>,
    target_pubkey: &[u8; 32],
) -> Result<WebSocketTransport> {
    let (ws_stream, _) = connect_async(rendezvous_url).await
        .with_context(|| format!("failed to connect to rendezvous {}", rendezvous_url))?;
    let mut transport = WebSocketTransport::new_tls(ws_stream);

    let target_hex = data_encoding::HEXLOWER.encode(target_pubkey);
    let mut conn_msg = json!({
        "action": "connect",
        "target": target_hex,
    });
    if let Some(tok) = token {
        conn_msg["token"] = json!(tok);
    }

    transport.send_text(&conn_msg.to_string()).await?;

    let resp = transport.recv_text().await?;
    let val: Value = serde_json::from_str(&resp)?;
    if val.get("status").and_then(|v| v.as_str()) != Some("joined") {
        let err = val.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(anyhow!("rendezvous error: {}", err));
    }

    transport.send_text(&json!({ "action": "ready" }).to_string()).await?;
    let active_resp = transport.recv_text().await?;
    let active_val: Value = serde_json::from_str(&active_resp)?;
    if active_val.get("status").and_then(|v| v.as_str()) != Some("relay_active") {
        return Err(anyhow!("failed to activate relay"));
    }

    Ok(transport)
}

pub struct RendezvousClient {
    pub url: String,
    pub token: Option<String>,
}

impl RendezvousClient {
    pub fn new(url: &str, token: Option<&str>) -> Self {
        Self {
            url: url.to_string(),
            token: token.map(|s| s.to_string()),
        }
    }

    pub async fn list_peers(&self, timeout: std::time::Duration) -> Result<Vec<super::discovery::DiscoveredPeer>> {
        let (ws_stream, _) = tokio::time::timeout(timeout, connect_async(&self.url))
            .await
            .map_err(|_| anyhow!("Connection timeout"))?
            .with_context(|| format!("failed to connect to rendezvous {}", self.url))?;
        let mut transport = WebSocketTransport::new_tls(ws_stream);

        let mut list_msg = json!({
            "action": "list",
        });
        if let Some(tok) = &self.token {
            list_msg["token"] = json!(tok);
        }

        transport.send_text(&list_msg.to_string()).await?;
        let resp = tokio::time::timeout(timeout, transport.recv_text())
            .await
            .map_err(|_| anyhow!("Response timeout"))??;
        let val: Value = serde_json::from_str(&resp)?;

        let mut peers = Vec::new();
        if let Some(arr) = val.get("peers").and_then(|v| v.as_array()) {
            for p in arr {
                let pk_hex = p.get("public_key").and_then(|v| v.as_str()).unwrap_or("");
                let pk_bytes = data_encoding::HEXLOWER.decode(pk_hex.as_bytes()).unwrap_or_default();
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let desc = p.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let fp = crate::protocol::storage::fingerprint(&pk_bytes);
                peers.push(super::discovery::DiscoveredPeer {
                    name: if name.is_empty() { format!("peer-{}", &pk_hex[..6.min(pk_hex.len())]) } else { name },
                    host: "rendezvous".to_string(),
                    port: 0,
                    public_key: pk_bytes,
                    fingerprint: fp,
                    description: desc,
                });
            }
        }
        Ok(peers)
    }
}
