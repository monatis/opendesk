//! opendesk server implementation: pairing listener and long-lived daemon.

use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::admin::{ActiveSessionEntry, AdminServer, SessionRegistry};
use super::audit::AuditLog;
use super::discovery::{Advertisement, advertise};
use super::transport::WebSocketTransport;
use crate::computer::local::LocalComputer;
use crate::protocol::crypto::EncryptedChannel;
use crate::protocol::frames::{Frame, HelloFrame, PushFrame, ResFrame, rmpv_to_json, value_get};
use crate::protocol::handshake::{Transport, auth_server, pair_server};
use crate::protocol::identity::Identity;
use crate::protocol::storage::{TrustedPeers, default_peer_name, fingerprint, read_description};

pub const DEFAULT_PORT: u16 = 8423;

#[derive(Clone, Debug)]
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
    registry: SessionRegistry,
    audit: AuditLog,
    advertise_mdns: bool,
    no_audit: bool,
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
        let registry = SessionRegistry::new();
        let audit = AuditLog::new(home);

        Ok(Self {
            host: host.to_string(),
            port,
            home: home.map(|p| p.to_path_buf()),
            identity,
            trusted,
            computer,
            registry,
            audit,
            advertise_mdns: true,
            no_audit: false,
            rendezvous_urls,
            rendezvous_token,
        })
    }

    pub fn set_advertise_mdns(&mut self, advertise: bool) {
        self.advertise_mdns = advertise;
    }

    pub fn set_no_audit(&mut self, no_audit: bool) {
        self.no_audit = no_audit;
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn registry(&self) -> &SessionRegistry {
        &self.registry
    }

    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    pub fn trusted(&self) -> &TrustedPeers {
        &self.trusted
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Enable pairing for a given code and timeout, returning the newly paired peer's public key.
    pub async fn enable_pairing(&self, code: &str, timeout_secs: f64) -> Result<Option<[u8; 32]>> {
        match tokio::time::timeout(
            Duration::from_secs_f64(timeout_secs),
            self.run_pair(code, timeout_secs as u64),
        )
        .await
        {
            Ok(Ok(pubkey)) => Ok(Some(pubkey)),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// Run one-shot pairing mode: accepts exactly one peer who proves the code, then terminates.
    pub async fn run_pair(&self, code: &str, timeout_secs: u64) -> Result<[u8; 32]> {
        let bind_addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .with_context(|| format!("failed to bind to {}", bind_addr))?;

        let fp = fingerprint(&self.identity.public_bytes());
        let pk_hex = data_encoding::HEXLOWER.encode(&self.identity.public_bytes());
        let peer_prefix = format!("peer-{}", &pk_hex[..6]);

        // Advertise via mDNS if enabled
        let mut _ad: Option<Advertisement> = None;
        if self.advertise_mdns {
            let desc = read_description(self.home.as_deref());
            let machine_name = std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "opendesk".to_string());
            match advertise(
                &machine_name,
                self.port,
                &self.identity.public_bytes(),
                &desc,
            ) {
                Ok(adv) => {
                    info!(
                        "mDNS advertisement started for '{}' on port {}",
                        machine_name, self.port
                    );
                    _ad = Some(adv);
                }
                Err(e) => warn!("mDNS advertisement failed in pairing mode: {}", e),
            }
        }

        let primary_rendezvous = self.rendezvous_urls.first().cloned();

        println!();
        println!("┌────────────────────────────────────────────────────────┐");
        println!("│  opendesk pairing                                      │");
        if let Some(ref r_url) = primary_rendezvous {
            println!("│  relay:       {:<41}│", r_url);
            println!("│  peer alias:  {:<41}│", peer_prefix);
        } else {
            println!("│  port:        {:<41}│", self.port);
        }
        println!("│  fingerprint: {:<41}│", fp);
        println!("│                                                        │");
        println!("│   pairing code:   {:<37}│", code);
        println!("│                                                        │");
        println!("│  Run on the controller:                                │");
        if let Some(ref r_url) = primary_rendezvous {
            println!("│    opendesk pair-with {} {:<21}│", peer_prefix, code);
            println!("│      --rendezvous {:<37}│", r_url);
        } else {
            println!("│    opendesk pair-with <host> {:<26}│", code);
        }
        println!("└────────────────────────────────────────────────────────┘");
        println!();

        // If rendezvous is configured, open control WebSocket to register
        let r_control: Option<(String, WebSocketTransport)> = if let Some(ref r_url) = primary_rendezvous {
            match tokio_tungstenite::connect_async(r_url).await {
                Ok((ws, _)) => {
                    let mut tr = WebSocketTransport::new_tls(ws);
                    let desc = read_description(self.home.as_deref());
                    let mut reg_msg = serde_json::json!({
                        "action": "register",
                        "public_key": pk_hex,
                        "name": peer_prefix,
                        "description": desc,
                    });
                    if let Some(tok) = &self.rendezvous_token {
                        reg_msg["token"] = serde_json::json!(tok);
                    }
                    if let Err(e) = tr.send_text(&reg_msg.to_string()).await {
                        warn!("Failed to register on rendezvous for pairing: {}", e);
                        None
                    } else {
                        let _ = tr.recv_text().await;
                        info!("Registered on rendezvous for pairing: {}", r_url);
                        Some((r_url.clone(), tr))
                    }
                }
                Err(e) => {
                    warn!("Failed to connect to rendezvous {}: {}", r_url, e);
                    None
                }
            }
        } else {
            None
        };

        let pair_code = code.to_string();
        let pair_future = async {
            let mut r_opt = r_control;
            let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
            ping_interval.tick().await;

            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        let (stream, peer_addr) = accept_res?;
                        let ws_stream = tokio_tungstenite::accept_async(stream).await?;
                        let mut transport = WebSocketTransport::new_plain(ws_stream);

                        match pair_server(&mut transport, &self.identity, &pair_code).await {
                            Ok(session) => {
                                let peer_pub = session.peer_public;
                                let peer_name = default_peer_name(&peer_pub);
                                self.trusted.add(&peer_pub, &peer_name, "")?;
                                self.trusted.cache_endpoint(
                                    &peer_pub,
                                    &peer_addr.ip().to_string(),
                                    peer_addr.port(),
                                )?;

                                let peer_fp = fingerprint(&peer_pub);
                                println!("✓ Paired with {} ({})", peer_name, peer_fp);

                                let comp = self.computer.clone();
                                let home = self.home.clone();
                                let audit = self.audit.clone();
                                let no_audit = self.no_audit;
                                tokio::spawn(async move {
                                    let (_tx, mut rx) = mpsc::channel(1);
                                    let _ = run_session_loop(
                                        &mut transport,
                                        session.channel,
                                        comp,
                                        home.as_deref(),
                                        &mut rx,
                                        &peer_pub,
                                        &peer_name,
                                        "pairing-session",
                                        &audit,
                                        no_audit,
                                    )
                                    .await;
                                });

                                return Ok(peer_pub);
                            }
                            Err(e) => {
                                warn!("Pairing attempt failed from {}: {}", peer_addr, e);
                            }
                        }
                    }
                    r_msg = async {
                        if let Some((_, ref mut tr)) = r_opt {
                            tr.recv_text().await
                        } else {
                            std::future::pending().await
                        }
                    } => {
                        match r_msg {
                            Ok(text) => {
                                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
                                    && val.get("action").and_then(|v| v.as_str()) == Some("session_request")
                                    && let Some(session_id) = val.get("session_id").and_then(|v| v.as_str())
                                {
                                    let r_url = r_opt.as_ref().unwrap().0.clone();
                                    info!("Incoming rendezvous session request for pairing: {}", session_id);
                                    let (join_ws, _) = tokio_tungstenite::connect_async(&r_url).await?;
                                    let mut join_tr = WebSocketTransport::new_tls(join_ws);
                                    let mut join_msg = serde_json::json!({
                                        "action": "join",
                                        "session_id": session_id,
                                    });
                                    if let Some(tok) = &self.rendezvous_token {
                                        join_msg["token"] = serde_json::json!(tok);
                                    }
                                    join_tr.send_text(&join_msg.to_string()).await?;
                                    let _ = join_tr.recv_text().await?;
                                    join_tr.send_text(&serde_json::json!({ "action": "ready" }).to_string()).await?;
                                    let _ = join_tr.recv_text().await?;

                                    // Now perform pair_server over rendezvous relay!
                                    match pair_server(&mut join_tr, &self.identity, &pair_code).await {
                                        Ok(session) => {
                                            let peer_pub = session.peer_public;
                                            let peer_name = default_peer_name(&peer_pub);
                                            self.trusted.add(&peer_pub, &peer_name, &r_url)?;
                                            let peer_fp = fingerprint(&peer_pub);
                                            println!("✓ Paired with {} ({}) via rendezvous relay", peer_name, peer_fp);
                                            return Ok(peer_pub);
                                        }
                                        Err(e) => {
                                            warn!("Rendezvous pairing handshake failed: {}", e);
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                warn!("Rendezvous control connection closed");
                                r_opt = None;
                            }
                        }
                    }
                    _ = ping_interval.tick() => {
                        if let Some((_, ref mut tr)) = r_opt {
                            let _ = tr.send_text(&serde_json::json!({ "action": "ping" }).to_string()).await;
                        }
                    }
                }
            }
        };

        match tokio::time::timeout(Duration::from_secs(timeout_secs), pair_future).await {
            Ok(res) => res,
            Err(_) => Err(anyhow!("pairing timed out after {} seconds", timeout_secs)),
        }
    }

    /// Run long-lived server daemon, accepting connections from trusted peers.
    pub async fn serve_forever(&self) -> Result<()> {
        if self.trusted.list().is_empty() {
            return Err(anyhow!("no trusted peers yet. Run `opendesk pair` first."));
        }

        // Start AdminServer for local IPC
        let mut admin_server = AdminServer::new(self.registry.clone(), self.home.as_deref());
        if let Err(e) = admin_server.start().await {
            warn!("Failed to start AdminServer: {}", e);
        }

        // Advertise via mDNS if enabled
        let mut _ad: Option<Advertisement> = None;
        if self.advertise_mdns {
            let desc = read_description(self.home.as_deref());
            let machine_name = std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "opendesk".to_string());
            match advertise(
                &machine_name,
                self.port,
                &self.identity.public_bytes(),
                &desc,
            ) {
                Ok(adv) => _ad = Some(adv),
                Err(e) => warn!("mDNS advertisement failed: {}", e),
            }
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
            let home = self.home.clone();
            let registry = self.registry.clone();
            let audit = self.audit.clone();
            let no_audit = self.no_audit;

            tokio::spawn(async move {
                if let Err(e) = run_rendezvous_serve_agent(
                    &r_url,
                    token.as_deref(),
                    identity,
                    trusted,
                    computer,
                    registry,
                    audit,
                    no_audit,
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
            let registry = self.registry.clone();
            let audit = self.audit.clone();
            let no_audit = self.no_audit;
            let home = self.home.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_inbound_connection(
                    stream,
                    remote_addr,
                    identity,
                    trusted,
                    computer,
                    registry,
                    audit,
                    no_audit,
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_inbound_connection(
    stream: tokio::net::TcpStream,
    remote_addr: SocketAddr,
    identity: Identity,
    trusted: TrustedPeers,
    computer: Arc<LocalComputer>,
    registry: SessionRegistry,
    audit: AuditLog,
    no_audit: bool,
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
    let peer_name = peer_entry
        .map(|p| p.name)
        .unwrap_or_else(|| default_peer_name(&session.peer_public));
    let session_id = uuid_short();
    let start_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    // Enforce single controller
    let existing_sessions = registry.list().await;
    if !existing_sessions.is_empty() {
        let existing = &existing_sessions[0];
        let existing_pub = data_encoding::HEXLOWER
            .decode(existing.peer_pubkey_hex.as_bytes())
            .unwrap_or_default();
        if existing_pub != session.peer_public {
            info!(
                "Rejecting controller {} — busy with {}",
                peer_name, existing.peer_name
            );
            if !no_audit {
                audit
                    .record_session_rejected(
                        &session.peer_public,
                        &peer_name,
                        &remote_addr.to_string(),
                        &format!("busy: active session {}", existing.peer_name),
                    )
                    .await;
            }
            // Reject with busy
            let reject_res = ResFrame::error(
                0,
                "busy",
                format!(
                    "server is busy: {} is the active controller",
                    existing.peer_name
                ),
            );
            let packed = rmp_serde::to_vec_named(&reject_res)?;
            let mut chan = session.channel;
            let ct = chan.encrypt(&packed)?;
            let _ = transport.send(&ct).await;
            return Ok(());
        } else {
            info!(
                "Same peer {} reconnecting; replacing previous session",
                peer_name
            );
            registry.kill_all("reconnected").await;
        }
    }

    let (evict_tx, mut evict_rx) = mpsc::channel(2);
    registry
        .add(ActiveSessionEntry {
            id: session_id.clone(),
            peer_name: peer_name.clone(),
            peer_public: session.peer_public,
            remote_addr: remote_addr.to_string(),
            started_at: start_ts,
            mode: "direct".to_string(),
            evict_tx,
        })
        .await;

    if !no_audit {
        audit
            .record_session_opened(
                &session.peer_public,
                &peer_name,
                &session_id,
                &remote_addr.to_string(),
                "direct",
            )
            .await;
    }

    info!(
        "Session {} established with peer '{}' ({})",
        session_id, peer_name, remote_addr
    );

    let start_time = Instant::now();
    let run_res = run_session_loop(
        &mut transport,
        session.channel,
        computer,
        home.as_deref(),
        &mut evict_rx,
        &session.peer_public,
        &peer_name,
        &session_id,
        &audit,
        no_audit,
    )
    .await;

    let duration = start_time.elapsed().as_secs_f64();
    registry.remove(&session_id).await;

    if !no_audit {
        audit
            .record_session_closed(
                &session.peer_public,
                &peer_name,
                &session_id,
                duration,
                if run_res.is_ok() { "normal" } else { "error" },
            )
            .await;
    }

    info!("Session {} closed for peer '{}'", session_id, peer_name);
    run_res
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_session_loop<T: Transport + ?Sized>(
    transport: &mut T,
    mut channel: EncryptedChannel,
    computer: Arc<LocalComputer>,
    home: Option<&Path>,
    evict_rx: &mut mpsc::Receiver<String>,
    peer_public: &[u8; 32],
    peer_name: &str,
    session_id: &str,
    audit: &AuditLog,
    no_audit: bool,
) -> Result<()> {
    // 1. Send HelloFrame with capabilities and description
    let mut caps = HashMap::new();
    caps.insert("display.capture".to_string(), rmpv::Value::Boolean(true));
    caps.insert("input.pointer".to_string(), rmpv::Value::Boolean(true));
    caps.insert("input.key".to_string(), rmpv::Value::Boolean(true));
    caps.insert("input.text".to_string(), rmpv::Value::Boolean(true));
    caps.insert("apps.open".to_string(), rmpv::Value::Boolean(true));
    caps.insert("apps.close".to_string(), rmpv::Value::Boolean(true));
    caps.insert("apps.focus".to_string(), rmpv::Value::Boolean(true));
    caps.insert("apps.list".to_string(), rmpv::Value::Boolean(true));
    caps.insert("ui.tree".to_string(), rmpv::Value::Boolean(true));
    caps.insert("ui.action".to_string(), rmpv::Value::Boolean(true));
    caps.insert("clipboard.read".to_string(), rmpv::Value::Boolean(true));
    caps.insert("clipboard.write".to_string(), rmpv::Value::Boolean(true));

    let desc = read_description(home);
    if !desc.is_empty() {
        caps.insert("description".to_string(), rmpv::Value::from(desc));
    }

    let hello = Frame::Hello(HelloFrame::server(caps));
    let hello_bytes = hello.to_msgpack()?;
    let hello_ct = channel.encrypt(&hello_bytes)?;
    transport.send(&hello_ct).await?;

    // 2. Request / Response loop with eviction support
    loop {
        tokio::select! {
            eviction_reason = evict_rx.recv() => {
                let reason = eviction_reason.unwrap_or_else(|| "admin_disconnect".to_string());
                info!("Session {} evicted: {}", session_id, reason);
                let mut payload = HashMap::new();
                payload.insert("reason".to_string(), rmpv::Value::from(reason));
                let push = Frame::Push(PushFrame::new("session.evicted", payload));
                if let Ok(bytes) = push.to_msgpack()
                    && let Ok(ct) = channel.encrypt(&bytes) {
                        let _ = transport.send(&ct).await;
                    }
                return Ok(());
            }
            res = transport.recv() => {
                let encrypted_frame = match res {
                    Ok(b) => b,
                    Err(_) => return Ok(()),
                };

                let decrypted_bytes = channel.decrypt(&encrypted_frame)
                    .context("failed to decrypt incoming frame")?;

                let frame = Frame::from_msgpack(&decrypted_bytes)
                    .context("malformed frame msgpack")?;

                match frame {
                    Frame::Req(req) => {
                        let res = dispatch_call(&computer, &req.method, &req.params).await;
                        let (res_frame, outcome, err_code, err_msg) = match res {
                            Ok(val) => (ResFrame::ok(req.id, val), "ok", None, None),
                            Err(err) => (ResFrame::error(req.id, "error", err.clone()), "error", Some("error"), Some(err)),
                        };

                        if !no_audit {
                            let params_val = rmpv_to_json(&rmpv::Value::Map(
                                req.params.iter().map(|(k, v)| (rmpv::Value::from(k.as_str()), v.clone())).collect()
                            ));
                            audit.record_call(
                                peer_public,
                                peer_name,
                                session_id,
                                &req.method,
                                &params_val,
                                outcome,
                                err_code,
                                err_msg.as_deref(),
                            ).await;
                        }

                        let res_bytes = Frame::Res(res_frame).to_msgpack()?;
                        let ct = channel.encrypt(&res_bytes)?;
                        transport.send(&ct).await?;
                    }
                    Frame::Hello(_) => {}
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
    }
}

pub(crate) async fn dispatch_call(
    computer: &LocalComputer,
    method: &str,
    params: &HashMap<String, rmpv::Value>,
) -> Result<rmpv::Value, String> {
    match method {
        "display.capture" => {
            let fmt = params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("jpeg");
            let quality = params
                .get("quality")
                .and_then(|v| v.as_i64())
                .unwrap_or(75) as u8;
            let max_dim = params
                .get("max_dim")
                .and_then(|v| v.as_i64())
                .map(|d| d as u32);
            let (bytes, mime, width, height) = computer
                .screenshot_format(None, fmt, quality, max_dim)
                .map_err(|e| e.to_string())?;
            // Transmit pure native MessagePack binary bytes (bin) - matching Python Pixmap
            Ok(rmpv::Value::Map(vec![
                (rmpv::Value::from("data"), rmpv::Value::Binary(bytes)),
                (rmpv::Value::from("format"), rmpv::Value::from(mime)),
                (rmpv::Value::from("width"), rmpv::Value::from(width as i64)),
                (rmpv::Value::from("height"), rmpv::Value::from(height as i64)),
            ]))
        }
        "input.pointer" => {
            let evt = params.get("event");
            let action = evt
                .and_then(|e| value_get(e, "action"))
                .and_then(|v| v.as_str())
                .unwrap_or("move");
            let pt = evt.and_then(|e| value_get(e, "point"));
            let x = pt
                .and_then(|p| value_get(p, "x"))
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                .unwrap_or(0.0) as i32;
            let y = pt
                .and_then(|p| value_get(p, "y"))
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                .unwrap_or(0.0) as i32;
            match action {
                "click" => {
                    let btn = evt
                        .and_then(|e| value_get(e, "button"))
                        .and_then(|v| v.as_str());
                    computer
                        .mouse_click(x, y, btn)
                        .map(|_| rmpv::Value::Nil)
                        .map_err(|e| e.to_string())
                }
                "move" => computer
                    .mouse_move(x, y)
                    .map(|_| rmpv::Value::Nil)
                    .map_err(|e| e.to_string()),
                "scroll" => {
                    let dy = evt
                        .and_then(|e| value_get(e, "dy"))
                        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                        .unwrap_or(0.0) as i32;
                    computer
                        .mouse_scroll(x, y, dy)
                        .map(|_| rmpv::Value::Nil)
                        .map_err(|e| e.to_string())
                }
                "down" => {
                    let btn = evt
                        .and_then(|e| value_get(e, "button"))
                        .and_then(|v| v.as_str());
                    computer
                        .mouse_down(btn)
                        .map(|_| rmpv::Value::Nil)
                        .map_err(|e| e.to_string())
                }
                "up" => {
                    let btn = evt
                        .and_then(|e| value_get(e, "button"))
                        .and_then(|v| v.as_str());
                    computer
                        .mouse_up(btn)
                        .map(|_| rmpv::Value::Nil)
                        .map_err(|e| e.to_string())
                }
                _ => computer
                    .mouse_click(x, y, None)
                    .map(|_| rmpv::Value::Nil)
                    .map_err(|e| e.to_string()),
            }
        }
        "input.text" => {
            let text = params
                .get("text")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    params
                        .get("text_input")
                        .and_then(|ti| value_get(ti, "text"))
                        .and_then(|v| v.as_str())
                })
                .ok_or("missing 'text'")?;
            computer
                .keyboard_type(text)
                .map(|_| rmpv::Value::Nil)
                .map_err(|e| e.to_string())
        }
        "input.key" => {
            let key = params
                .get("key")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    params
                        .get("event")
                        .and_then(|e| value_get(e, "keysym"))
                        .and_then(|v| v.as_str())
                })
                .ok_or("missing 'key'")?;
            let action = params
                .get("event")
                .and_then(|e| value_get(e, "action"))
                .and_then(|v| v.as_str())
                .or_else(|| params.get("action").and_then(|v| v.as_str()))
                .unwrap_or("press");
            match action {
                "down" => computer
                    .keyboard_down(key)
                    .map(|_| rmpv::Value::Nil)
                    .map_err(|e| e.to_string()),
                "up" => computer
                    .keyboard_up(key)
                    .map(|_| rmpv::Value::Nil)
                    .map_err(|e| e.to_string()),
                _ => {
                    if key.contains('+') || key.contains('-') {
                        let parts: Vec<&str> = key
                            .split(['+', '-'])
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .collect();
                        computer
                            .keyboard_hotkey(&parts)
                            .map(|_| rmpv::Value::Nil)
                            .map_err(|e| e.to_string())
                    } else {
                        computer
                            .keyboard_press(key)
                            .map(|_| rmpv::Value::Nil)
                            .map_err(|e| e.to_string())
                    }
                }
            }
        }
        "apps.open" => {
            let path = params
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("path").and_then(|v| v.as_str()))
                .ok_or("missing 'name'")?;
            computer
                .app_open(path)
                .map(|_| rmpv::Value::Nil)
                .map_err(|e| e.to_string())
        }
        "apps.close" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("missing 'name'")?;
            computer
                .app_close(name)
                .map(|_| rmpv::Value::Nil)
                .map_err(|e| e.to_string())
        }
        "apps.focus" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("missing 'name'")?;
            computer
                .app_focus(name)
                .map(|_| rmpv::Value::Nil)
                .map_err(|e| e.to_string())
        }
        "apps.list" | "windows.list" => {
            let apps = computer.app_list().map_err(|e| e.to_string())?;
            let items: Vec<rmpv::Value> = apps
                .iter()
                .map(|s| rmpv::Value::from(s.name.as_str()))
                .collect();
            Ok(rmpv::Value::Map(vec![(
                rmpv::Value::from("items"),
                rmpv::Value::Array(items),
            )]))
        }
        "ui.tree" => {
            let app_name = params
                .get("app")
                .or_else(|| params.get("app_name"))
                .and_then(|v| v.as_str());
            let max_depth = params
                .get("max_depth")
                .and_then(|v| v.as_u64())
                .map(|d| d as usize);
            let tree = computer
                .ui_tree(app_name, max_depth)
                .map_err(|e| e.to_string())?;
            Ok(rmpv::Value::Map(vec![(
                rmpv::Value::from("tree"),
                rmpv::Value::from(tree),
            )]))
        }
        "ui.action" => {
            let app_name = params
                .get("app")
                .or_else(|| params.get("app_name"))
                .and_then(|v| v.as_str());
            let selector = params
                .get("element")
                .and_then(|e| value_get(e, "name"))
                .and_then(|v| v.as_str())
                .or_else(|| params.get("selector").and_then(|v| v.as_str()))
                .or_else(|| params.get("name").and_then(|v| v.as_str()))
                .ok_or("missing 'element.name' or 'selector'")?;
            computer
                .ui_click(app_name, selector)
                .map(|_| rmpv::Value::Nil)
                .map_err(|e| e.to_string())
        }
        "clipboard.read" => {
            let text = computer.clipboard_read().map_err(|e| e.to_string())?;
            let entry = rmpv::Value::Map(vec![
                (
                    rmpv::Value::from("mime_type"),
                    rmpv::Value::from("text/plain;charset=utf-8"),
                ),
                (
                    rmpv::Value::from("data"),
                    rmpv::Value::Binary(text.as_bytes().to_vec()),
                ),
            ]);
            Ok(rmpv::Value::Map(vec![
                (
                    rmpv::Value::from("entries"),
                    rmpv::Value::Array(vec![entry]),
                ),
                (rmpv::Value::from("text"), rmpv::Value::from(text)),
            ]))
        }
        "clipboard.write" => {
            let text = params
                .get("contents")
                .and_then(|c| value_get(c, "entries"))
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|e| value_get(e, "data"))
                .and_then(|d| {
                    d.as_slice()
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .or_else(|| d.as_str())
                })
                .or_else(|| params.get("text").and_then(|v| v.as_str()))
                .ok_or("missing 'contents'")?;
            computer
                .clipboard_write(text)
                .map(|_| rmpv::Value::Nil)
                .map_err(|e| e.to_string())
        }
        "system.capabilities" => {
            let caps = vec![
                rmpv::Value::from("display.capture"),
                rmpv::Value::from("input.pointer"),
                rmpv::Value::from("input.key"),
                rmpv::Value::from("input.text"),
                rmpv::Value::from("apps.open"),
                rmpv::Value::from("apps.close"),
                rmpv::Value::from("apps.focus"),
                rmpv::Value::from("apps.list"),
                rmpv::Value::from("ui.tree"),
                rmpv::Value::from("ui.action"),
                rmpv::Value::from("clipboard.read"),
                rmpv::Value::from("clipboard.write"),
            ];
            Ok(rmpv::Value::Map(vec![
                (rmpv::Value::from("capabilities"), rmpv::Value::Array(caps)),
                (rmpv::Value::from("limits"), rmpv::Value::Map(vec![])),
                (
                    rmpv::Value::from("protocol_version"),
                    rmpv::Value::from("0.1"),
                ),
                (rmpv::Value::from("backend"), rmpv::Value::from("xa11y")),
                (rmpv::Value::from("description"), rmpv::Value::from("")),
            ]))
        }
        "system.environment" => Ok(rmpv::Value::Map(vec![
            (
                rmpv::Value::from("os"),
                rmpv::Value::from(std::env::consts::OS),
            ),
            (rmpv::Value::from("os_version"), rmpv::Value::from("")),
            (rmpv::Value::from("hostname"), rmpv::Value::from("")),
            (rmpv::Value::from("locale"), rmpv::Value::from("")),
            (rmpv::Value::from("timezone"), rmpv::Value::from("")),
            (rmpv::Value::from("displays"), rmpv::Value::Array(vec![])),
        ])),
        other => Err(format!("unknown method: {}", other)),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_rendezvous_serve_agent(
    r_url: &str,
    token: Option<&str>,
    identity: Identity,
    trusted: TrustedPeers,
    computer: Arc<LocalComputer>,
    _registry: SessionRegistry,
    _audit: AuditLog,
    _no_audit: bool,
    home: Option<PathBuf>,
) -> Result<()> {
    use super::rendezvous::run_rendezvous_agent;
    // We bridge into the existing rendezvous agent
    let active = Arc::new(tokio::sync::Mutex::new(None));
    run_rendezvous_agent(r_url, token, identity, trusted, computer, active, home).await
}

fn uuid_short() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let n: u32 = rng.gen_range(0..=u32::MAX);
    format!("{:08x}", n)
}
