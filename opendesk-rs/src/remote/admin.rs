//! Local IPC for inspecting / killing active opendesk-serve sessions.
//!
//! A user-only channel — Unix domain socket at `~/.opendesk/admin.sock` (mode 0600),
//! or localhost TCP on a file-recorded port on Windows (`~/.opendesk/admin.port`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};

use crate::protocol::identity::default_home;

pub const SOCKET_NAME: &str = "admin.sock";
pub const PORT_FILE: &str = "admin.port";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub peer_name: String,
    pub peer_pubkey_hex: String,
    pub remote_addr: String,
    pub started_at: f64,
    pub age_seconds: f64,
    pub mode: String,
}

pub struct ActiveSessionEntry {
    pub id: String,
    pub peer_name: String,
    pub peer_public: [u8; 32],
    pub remote_addr: String,
    pub started_at: f64,
    pub mode: String,
    pub evict_tx: mpsc::Sender<String>,
}

#[derive(Clone, Default)]
pub struct SessionRegistry {
    sessions: Arc<Mutex<HashMap<String, ActiveSessionEntry>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn add(&self, entry: ActiveSessionEntry) {
        let mut map = self.sessions.lock().await;
        map.insert(entry.id.clone(), entry);
    }

    pub async fn remove(&self, session_id: &str) {
        let mut map = self.sessions.lock().await;
        map.remove(session_id);
    }

    pub async fn list(&self) -> Vec<SessionSnapshot> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let map = self.sessions.lock().await;
        map.values()
            .map(|s| SessionSnapshot {
                id: s.id.clone(),
                peer_name: s.peer_name.clone(),
                peer_pubkey_hex: data_encoding::HEXLOWER.encode(&s.peer_public),
                remote_addr: s.remote_addr.clone(),
                started_at: s.started_at,
                age_seconds: (now - s.started_at).max(0.0),
                mode: s.mode.clone(),
            })
            .collect()
    }

    pub async fn kill(&self, session_id: &str, reason: &str) -> bool {
        let entry = {
            let mut map = self.sessions.lock().await;
            map.remove(session_id)
        };
        if let Some(s) = entry {
            let _ = s.evict_tx.send(reason.to_string()).await;
            true
        } else {
            false
        }
    }

    pub async fn kill_all(&self, reason: &str) -> usize {
        let entries: Vec<ActiveSessionEntry> = {
            let mut map = self.sessions.lock().await;
            map.drain().map(|(_, v)| v).collect()
        };
        let count = entries.len();
        for s in entries {
            let _ = s.evict_tx.send(reason.to_string()).await;
        }
        count
    }
}

// ---------------------------------------------------------------------------
// Framing: 4-byte big-endian length + msgpack dict
// ---------------------------------------------------------------------------

async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Value> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 10 * 1024 * 1024 {
        return Err(anyhow!("frame too large: {} bytes", len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let val: Value = rmp_serde::from_slice(&buf)?;
    Ok(val)
}

async fn write_frame<W: AsyncWriteExt + Unpin>(writer: &mut W, val: &Value) -> Result<()> {
    let buf = rmp_serde::to_vec_named(val)?;
    let len = (buf.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// AdminServer
// ---------------------------------------------------------------------------

pub struct AdminServer {
    home: PathBuf,
    registry: SessionRegistry,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl AdminServer {
    pub fn new(registry: SessionRegistry, home: Option<&Path>) -> Self {
        let base = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
        Self {
            home: base,
            registry,
            shutdown_tx: None,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.home)?;

        #[cfg(windows)]
        {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let port = listener.local_addr()?.port();
            let port_path = self.home.join(PORT_FILE);
            std::fs::write(&port_path, port.to_string())?;

            let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
            self.shutdown_tx = Some(tx);
            let registry = self.registry.clone();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = &mut rx => break,
                        accept_res = listener.accept() => {
                            if let Ok((mut stream, _)) = accept_res {
                                let reg = registry.clone();
                                tokio::spawn(async move {
                                    let _ = handle_admin_stream(&mut stream, reg).await;
                                });
                            }
                        }
                    }
                }
            });
        }

        #[cfg(not(windows))]
        {
            let sock_path = self.home.join(SOCKET_NAME);
            let _ = std::fs::remove_file(&sock_path);
            let listener = tokio::net::UnixListener::bind(&sock_path)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600));
            }

            let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
            self.shutdown_tx = Some(tx);
            let registry = self.registry.clone();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = &mut rx => break,
                        accept_res = listener.accept() => {
                            if let Ok((mut stream, _)) = accept_res {
                                let reg = registry.clone();
                                tokio::spawn(async move {
                                    let _ = handle_admin_stream(&mut stream, reg).await;
                                });
                            }
                        }
                    }
                }
            });
        }

        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        #[cfg(windows)]
        {
            let port_path = self.home.join(PORT_FILE);
            let _ = std::fs::remove_file(port_path);
        }
        #[cfg(not(windows))]
        {
            let sock_path = self.home.join(SOCKET_NAME);
            let _ = std::fs::remove_file(sock_path);
        }
    }
}

impl Drop for AdminServer {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn handle_admin_stream<S>(stream: &mut S, registry: SessionRegistry) -> Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    loop {
        let req = match read_frame(stream).await {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };

        let op = req
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let res = match op {
            "list" => {
                let sessions = registry.list().await;
                json!({
                    "ok": true,
                    "sessions": sessions,
                })
            }
            "kill" => {
                let id = req.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let killed = registry.kill(id, "admin_disconnect").await;
                json!({
                    "ok": killed,
                    "killed": if killed { 1 } else { 0 },
                })
            }
            "kill_all" => {
                let count = registry.kill_all("admin_disconnect").await;
                json!({
                    "ok": true,
                    "killed": count,
                })
            }
            _ => json!({
                "ok": false,
                "error": format!("unknown op '{op}'"),
            }),
        };

        if write_frame(stream, &res).await.is_err() {
            return Ok(());
        }
    }
}

// ---------------------------------------------------------------------------
// AdminClient
// ---------------------------------------------------------------------------

pub struct AdminClient {
    #[cfg(windows)]
    stream: tokio::net::TcpStream,
    #[cfg(not(windows))]
    stream: tokio::net::UnixStream,
}

impl AdminClient {
    pub async fn connect(home: Option<&Path>) -> Result<Self> {
        let base = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);

        #[cfg(windows)]
        {
            let port_path = base.join(PORT_FILE);
            if !port_path.exists() {
                return Err(anyhow!(
                    "No opendesk admin endpoint at {}. Is `opendesk serve` running?",
                    port_path.display()
                ));
            }
            let port_str = std::fs::read_to_string(&port_path)
                .with_context(|| format!("Failed to read {}", port_path.display()))?;
            let port: u16 = port_str.trim().parse()
                .map_err(|e| anyhow!("Corrupt admin port file: {}", e))?;

            let stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
                .await
                .with_context(|| format!("Failed to connect to admin port {}", port))?;

            Ok(Self { stream })
        }

        #[cfg(not(windows))]
        {
            let sock_path = base.join(SOCKET_NAME);
            if !sock_path.exists() {
                return Err(anyhow!(
                    "No opendesk admin socket at {}. Is `opendesk serve` running?",
                    sock_path.display()
                ));
            }
            let stream = tokio::net::UnixStream::connect(&sock_path)
                .await
                .with_context(|| format!("Failed to connect to admin socket {}", sock_path.display()))?;

            Ok(Self { stream })
        }
    }

    pub async fn list_sessions(&mut self) -> Result<Vec<SessionSnapshot>> {
        let req = json!({ "op": "list" });
        write_frame(&mut self.stream, &req).await?;
        let res = read_frame(&mut self.stream).await?;

        let ok = res.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            let err = res
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("list failed");
            return Err(anyhow!("{}", err));
        }

        let sess_val = res.get("sessions").cloned().unwrap_or(Value::Array(vec![]));
        let list: Vec<SessionSnapshot> = serde_json::from_value(sess_val)?;
        Ok(list)
    }

    pub async fn kill(&mut self, session_id: &str) -> Result<bool> {
        let req = json!({ "op": "kill", "id": session_id });
        write_frame(&mut self.stream, &req).await?;
        let res = read_frame(&mut self.stream).await?;
        let ok = res.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        Ok(ok)
    }

    pub async fn kill_all(&mut self) -> Result<usize> {
        let req = json!({ "op": "kill_all" });
        write_frame(&mut self.stream, &req).await?;
        let res = read_frame(&mut self.stream).await?;
        let count = res.get("killed").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(count as usize)
    }
}

pub fn format_age(seconds: f64) -> String {
    let s = seconds as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86400)
    }
}
