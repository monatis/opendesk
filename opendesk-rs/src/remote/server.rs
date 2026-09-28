//! opendesk server implementation: pairing listener and long-lived daemon.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::computer::local::LocalComputer;
use crate::protocol::crypto::EncryptedChannel;
use crate::protocol::frames::{Frame, HelloFrame, ResFrame};
use crate::protocol::handshake::{auth_server, pair_server, Transport};
use crate::protocol::identity::Identity;
use crate::protocol::storage::{
    default_peer_name, fingerprint, read_description, TrustedPeers,
};
use super::transport::WebSocketTransport;

pub const DEFAULT_PORT: u16 = 8423;

#[derive(Clone)]
pub struct ActiveSession {
    pub session_id: String,
    pub peer_public: [u8; 32],
    pub peer_name: String,
    pub remote_addr: String,
}

pub struct OpendeskServer {
    host: String,
    port: u16,
    home: Option<PathBuf>,
    identity: Identity,
    trusted: TrustedPeers,
    computer: Arc<LocalComputer>,
    active_session: Arc<Mutex<Option<ActiveSession>>>,
    rendezvous_urls: Vec<String>,
    rendezvous_token: Option<String>,
}

impl OpendeskServer {
    pub fn new(
        host: &str,
        port: u16,
        home: Option<&Path>,
        rendezvous_urls: Vec<String>,
        rendezvous_token: Option<String>,
    ) -> Result<Self> {
        let identity = Identity::load_or_create(home)?;
        let trusted = TrustedPeers::new(home);
        let computer = Arc::new(LocalComputer::new());

        Ok(Self {
            host: host.to_string(),
            port,
            home: home.map(|p| p.to_path_buf()),
            identity,
            trusted,
            computer,
            active_session: Arc::new(Mutex::new(None)),
            rendezvous_urls,
            rendezvous_token,
        })
    }

    /// Run one-shot pairing mode: accepts exactly one peer who proves the code, then terminates.
    pub async fn run_pair(&self, code: &str, timeout_secs: u64) -> Result<[u8; 32]> {
        let bind_addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("failed to bind to {}", bind_addr))?;

        let fp = fingerprint(&self.identity.public_bytes());

        println!();
        println!("┌──────────────────────────────────────────────┐");
        println!("│  opendesk pairing                            │");
        println!("│  port:        {:<31}│", self.port);
        println!("│  fingerprint: {:<31}│", fp);
        println!("│                                              │");
        println!("│   pairing code:   {:<27}│", code);
        println!("│                                              │");
        println!("│  Run on the controller:                      │");
        println!("│    opendesk pair-with <host> {:<16}│", code);
        println!("└──────────────────────────────────────────────┘");
        println!();

        let pair_future = async {
            loop {
                let (stream, peer_addr) = listener.accept().await?;
                let ws_stream = tokio_tungstenite::accept_async(stream).await?;
                let mut transport = WebSocketTransport::new_plain(ws_stream);

                match pair_server(&mut transport, &self.identity, code).await {
                    Ok(session) => {
                        let peer_pub = session.peer_public;
                        let peer_name = default_peer_name(&peer_pub);
                        self.trusted.add(&peer_pub, &peer_name, "")?;
                        self.trusted.cache_endpoint(&peer_pub, &peer_addr.ip().to_string(), peer_addr.port())?;

                        let peer_fp = fingerprint(&peer_pub);
                        println!("✓ Paired with {} ({})", peer_name, peer_fp);

                        let comp = self.computer.clone();
                        let home = self.home.clone();
                        tokio::spawn(async move {
                            let _ = run_session_loop(&mut transport, session.channel, comp, home.as_deref()).await;
                        });

                        return Ok(peer_pub);
                    }
                    Err(e) => {
                        warn!("Pairing attempt failed from {}: {}", peer_addr, e);
                    }
                }
            }
        };

        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), pair_future).await {
            Ok(res) => res,
            Err(_) => Err(anyhow!("pairing timed out after {} seconds", timeout_secs)),
        }
    }

    /// Run long-lived server daemon, accepting connections from trusted peers.
    pub async fn serve_forever(&self) -> Result<()> {
        if self.trusted.list().is_empty() {
            return Err(anyhow!(
                "no trusted peers yet. Run `opendesk pair` first."
            ));
        }

        let bind_addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("failed to bind to {}", bind_addr))?;

        let fp = fingerprint(&self.identity.public_bytes());
        info!("opendesk serve listening on {} (fp={})", bind_addr, fp);
        println!("opendesk serve listening on {} (fp={})", bind_addr, fp);

        // Spawn outbound rendezvous agents if configured
        for r_url in &self.rendezvous_urls {
            let r_url = r_url.clone();
            let token = self.rendezvous_token.clone();
            let identity = self.identity.clone();
            let trusted = self.trusted.clone();
            let computer = self.computer.clone();
            let active = self.active_session.clone();
            let home = self.home.clone();

            tokio::spawn(async move {
                if let Err(e) = super::rendezvous::run_rendezvous_agent(
                    &r_url,
                    token.as_deref(),
                    identity,
                    trusted,
                    computer,
                    active,
                    home,
                )
                .await
                {
                    warn!("Rendezvous agent for {} stopped: {}", r_url, e);
                }
            });
        }

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            let identity = self.identity.clone();
            let trusted = self.trusted.clone();
            let computer = self.computer.clone();
            let active_session = self.active_session.clone();
            let home = self.home.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_inbound_connection(
                    stream,
                    remote_addr,
                    identity,
                    trusted,
                    computer,
                    active_session,
                    home,
                )
                .await
                {
                    warn!("Session error from {}: {}", remote_addr, e);
                }
            });
        }
    }
}

pub(crate) async fn handle_inbound_connection(
    stream: tokio::net::TcpStream,
    remote_addr: SocketAddr,
    identity: Identity,
    trusted: TrustedPeers,
    computer: Arc<LocalComputer>,
    active_session: Arc<Mutex<Option<ActiveSession>>>,
    home: Option<PathBuf>,
) -> Result<()> {
    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    let mut transport = WebSocketTransport::new_plain(ws_stream);

    // Authenticate client
    let session = match auth_server(&mut transport, &identity, &trusted).await {
        Ok(s) => s,
        Err(e) => {
            warn!("Auth rejected from {}: {}", remote_addr, e);
            return Err(e);
        }
    };

    let peer_entry = trusted.find(&session.peer_public);
    let peer_name = peer_entry.map(|p| p.name).unwrap_or_else(|| default_peer_name(&session.peer_public));
    let session_id = uuid_short();

    // Enforce single controller
    {
        let mut active = active_session.lock().await;
        if let Some(existing) = active.as_ref() {
            if existing.peer_public != session.peer_public {
                info!("Rejecting controller {} — busy with {}", peer_name, existing.peer_name);
                // Reject with busy
                let reject_res = ResFrame::error(0, "busy", format!("server is busy: {} is the active controller", existing.peer_name));
                let packed = rmp_serde::to_vec_named(&reject_res)?;
                let mut chan = session.channel;
                let ct = chan.encrypt(&packed)?;
                let _ = transport.send(&ct).await;
                return Ok(());
            } else {
                info!("Same peer {} reconnecting; replacing previous session", peer_name);
            }
        }
        *active = Some(ActiveSession {
            session_id: session_id.clone(),
            peer_public: session.peer_public,
            peer_name: peer_name.clone(),
            remote_addr: remote_addr.to_string(),
        });
    }

    info!("Session {} established with peer '{}' ({})", session_id, peer_name, remote_addr);

    // Session worker
    let run_res = run_session_loop(
        &mut transport,
        session.channel,
        computer,
        home.as_deref(),
    )
    .await;

    // Clear active session
    {
        let mut active = active_session.lock().await;
        if let Some(s) = active.as_ref() {
            if s.session_id == session_id {
                *active = None;
            }
        }
    }

    info!("Session {} closed for peer '{}'", session_id, peer_name);
    run_res
}

pub(crate) async fn run_session_loop<T: Transport + ?Sized>(
    transport: &mut T,
    mut channel: EncryptedChannel,
    computer: Arc<LocalComputer>,
    home: Option<&Path>,
) -> Result<()> {
    // 1. Send HelloFrame with capabilities and description
    let mut caps = HashMap::new();
    caps.insert("mouse".to_string(), json!(true));
    caps.insert("keyboard".to_string(), json!(true));
    caps.insert("app".to_string(), json!(true));
    caps.insert("ui".to_string(), json!(true));
    caps.insert("clipboard".to_string(), json!(true));
    caps.insert("screenshot".to_string(), json!(true));

    let desc = read_description(home);
    if !desc.is_empty() {
        caps.insert("description".to_string(), json!(desc));
    }

    let hello = Frame::Hello(HelloFrame::server(caps));
    let hello_bytes = hello.to_msgpack()?;
    let hello_ct = channel.encrypt(&hello_bytes)?;
    transport.send(&hello_ct).await?;

    // 2. Request / Response loop
    loop {
        let encrypted_frame = match transport.recv().await {
            Ok(b) => b,
            Err(_) => {
                // Normal close or disconnection
                return Ok(());
            }
        };

        let decrypted_bytes = channel.decrypt(&encrypted_frame)
            .context("failed to decrypt incoming frame")?;

        let frame = Frame::from_msgpack(&decrypted_bytes)
            .context("malformed frame msgpack")?;

        match frame {
            Frame::Req(req) => {
                let res = dispatch_call(&computer, &req.method, &req.params).await;
                let res_frame = match res {
                    Ok(val) => ResFrame::ok(req.id, val),
                    Err(err) => ResFrame::error(req.id, "error", err),
                };

                let res_bytes = Frame::Res(res_frame).to_msgpack()?;
                let ct = channel.encrypt(&res_bytes)?;
                transport.send(&ct).await?;
            }
            Frame::Hello(_) => {
                // Client hello received
            }
            Frame::Push(push) => {
                info!("Received push event: {}", push.topic);
            }
            Frame::Cancel(c) => {
                info!("Request cancelled: {}", c.id);
            }
            Frame::Res(_) => {
                warn!("Server received unexpected ResFrame");
            }
        }
    }
}

pub(crate) async fn dispatch_call(
    computer: &LocalComputer,
    method: &str,
    params: &HashMap<String, Value>,
) -> Result<Value, String> {
    match method {
        "computer.screenshot" => {
            let bytes = computer.screenshot(None).map_err(|e| e.to_string())?;
            let b64 = data_encoding::BASE64.encode(&bytes);
            Ok(json!({ "image_base64": b64, "format": "png" }))
        }
        "computer.mouse_move" => {
            let x = params.get("x").and_then(|v| v.as_i64()).ok_or("missing 'x'")? as i32;
            let y = params.get("y").and_then(|v| v.as_i64()).ok_or("missing 'y'")? as i32;
            computer.mouse_move(x, y).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.mouse_click" => {
            let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let button = params.get("button").and_then(|v| v.as_str());
            computer.mouse_click(x, y, button).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.mouse_double_click" => {
            let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            computer.mouse_double_click(x, y).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.mouse_right_click" => {
            let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            computer.mouse_click(x, y, Some("right")).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.mouse_drag" => {
            let sx = params.get("start_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let sy = params.get("start_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ex = params.get("end_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ey = params.get("end_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            computer.mouse_drag(sx, sy, ex, ey).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.mouse_scroll" => {
            let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let dy = params.get("delta_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            computer.mouse_scroll(x, y, dy).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.keyboard_type" => {
            let text = params.get("text").and_then(|v| v.as_str()).ok_or("missing 'text'")?;
            computer.keyboard_type(text).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.keyboard_press" => {
            let key = params.get("key").and_then(|v| v.as_str()).ok_or("missing 'key'")?;
            computer.keyboard_press(key).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.keyboard_hotkey" => {
            let keys_arr = params.get("keys").and_then(|v| v.as_array()).ok_or("missing 'keys'")?;
            let keys: Vec<&str> = keys_arr.iter().filter_map(|v| v.as_str()).collect();
            computer.keyboard_hotkey(&keys).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.app_open" => {
            let path = params.get("path")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("name").and_then(|v| v.as_str()))
                .ok_or("missing 'path'")?;
            computer.app_open(path).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.app_focus" => {
            let name = params.get("name").and_then(|v| v.as_str()).ok_or("missing 'name'")?;
            computer.app_focus(name).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.app_close" => {
            let name = params.get("name").and_then(|v| v.as_str()).ok_or("missing 'name'")?;
            computer.app_close(name).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.app_list" => {
            computer.app_list().map(|apps| json!({ "apps": apps })).map_err(|e| e.to_string())
        }
        "computer.ui_tree" => {
            let app_name = params.get("app_name").and_then(|v| v.as_str());
            let max_depth = params.get("max_depth").and_then(|v| v.as_u64()).map(|d| d as usize);
            computer.ui_tree(app_name, max_depth).map(|tree| json!({ "tree": tree })).map_err(|e| e.to_string())
        }
        "computer.ui_click" => {
            let app_name = params.get("app_name").and_then(|v| v.as_str());
            let selector = params.get("selector")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("name").and_then(|v| v.as_str()))
                .ok_or("missing 'selector'")?;
            computer.ui_click(app_name, selector).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.ui_type" => {
            let app_name = params.get("app_name").and_then(|v| v.as_str());
            let selector = params.get("selector")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("name").and_then(|v| v.as_str()))
                .ok_or("missing 'selector'")?;
            let text = params.get("text").and_then(|v| v.as_str()).ok_or("missing 'text'")?;
            computer.ui_type(app_name, selector, text).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        "computer.clipboard_read" => {
            computer.clipboard_read().map(|text| json!({ "text": text })).map_err(|e| e.to_string())
        }
        "computer.clipboard_write" => {
            let text = params.get("text").and_then(|v| v.as_str()).ok_or("missing 'text'")?;
            computer.clipboard_write(text).map(|_| json!({ "status": "ok" })).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown method: {}", other)),
    }
}

fn uuid_short() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let n: u32 = rng.gen_range(0..=u32::MAX);
    format!("{:08x}", n)
}
