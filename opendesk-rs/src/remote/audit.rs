//! Server-side audit log.
//!
//! Every protocol method dispatched on the controlled machine, plus session
//! lifecycle events (open / close / rejected), is appended to a per-day JSONL
//! file under `<home>/audit/YYYY-MM-DD.jsonl`.

use chrono::Local;
use serde_json::{Value, json};
use std::fs::{OpenOptions, create_dir_all};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::warn;

use crate::protocol::storage::default_home;

pub const AUDIT_DIR_NAME: &str = "audit";

#[derive(Clone)]
pub struct AuditLog {
    dir: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl AuditLog {
    pub fn new(home: Option<&Path>) -> Self {
        let base = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
        let dir = base.join(AUDIT_DIR_NAME);
        let _ = create_dir_all(&dir);
        Self {
            dir,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    fn today_iso() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    fn now_ts() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    fn peer_id(public_key: &[u8], name: &str) -> Value {
        let hex = data_encoding::HEXLOWER.encode(public_key);
        let fp = if hex.len() >= 16 { &hex[..16] } else { &hex };
        json!({
            "name": name,
            "fp": fp,
        })
    }

    async fn write_entry(&self, mut entry: Value) {
        let _guard = self.lock.lock().await;
        if entry.get("ts").is_none()
            && let Some(obj) = entry.as_object_mut()
        {
            obj.insert("ts".to_string(), json!(Self::now_ts()));
        }

        let date = Self::today_iso();
        let path = self.dir.join(format!("{}.jsonl", date));

        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(mut file) => {
                if let Ok(line) = serde_json::to_string(&entry) {
                    let _ = writeln!(file, "{}", line);
                }
            }
            Err(e) => {
                warn!("Failed to write audit entry to {}: {}", path.display(), e);
            }
        }
    }

    pub async fn record_session_opened(
        &self,
        peer_public: &[u8],
        peer_name: &str,
        session_id: &str,
        remote_addr: &str,
        mode: &str,
    ) {
        self.write_entry(json!({
            "type": "session.opened",
            "peer": Self::peer_id(peer_public, peer_name),
            "session_id": session_id,
            "remote_addr": remote_addr,
            "mode": mode,
        }))
        .await;
    }

    pub async fn record_session_closed(
        &self,
        peer_public: &[u8],
        peer_name: &str,
        session_id: &str,
        duration: f64,
        reason: &str,
    ) {
        self.write_entry(json!({
            "type": "session.closed",
            "peer": Self::peer_id(peer_public, peer_name),
            "session_id": session_id,
            "duration": (duration * 1000.0).round() / 1000.0,
            "reason": reason,
        }))
        .await;
    }

    pub async fn record_session_rejected(
        &self,
        peer_public: &[u8],
        peer_name: &str,
        remote_addr: &str,
        reason: &str,
    ) {
        self.write_entry(json!({
            "type": "session.rejected",
            "peer": Self::peer_id(peer_public, peer_name),
            "remote_addr": remote_addr,
            "reason": reason,
        }))
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_call(
        &self,
        peer_public: &[u8],
        peer_name: &str,
        session_id: &str,
        method: &str,
        params: &Value,
        outcome: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) {
        let summary = summarise(method, params);
        let mut entry = json!({
            "type": "call",
            "peer": Self::peer_id(peer_public, peer_name),
            "session_id": session_id,
            "method": method,
            "summary": summary,
            "outcome": outcome,
        });

        if let Some(code) = error_code {
            entry["error_code"] = json!(code);
        }
        if let Some(msg) = error_message {
            let truncated = if msg.len() > 200 { &msg[..200] } else { msg };
            entry["error_message"] = json!(truncated);
        }

        self.write_entry(entry).await;
    }

    pub fn iter_entries(&self, date: Option<&str>) -> Vec<Value> {
        let d = date.map(|s| s.to_string()).unwrap_or_else(Self::today_iso);
        let path = self.dir.join(format!("{}.jsonl", d));
        if !path.exists() {
            return vec![];
        }

        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return vec![],
        };

        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for l in reader.lines().map_while(Result::ok) {
            let trimmed = l.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                entries.push(val);
            }
        }
        entries
    }
}

/// One-line natural description of an inbound method call.
pub fn summarise(method: &str, params: &Value) -> String {
    match method {
        "input.pointer" => {
            let evt = params.get("event").unwrap_or(&Value::Null);
            let pt = evt.get("point").unwrap_or(&Value::Null);
            let action = evt.get("action").and_then(|v| v.as_str()).unwrap_or("move");
            let x = pt
                .get("x")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into());
            let y = pt
                .get("y")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into());
            format!("send pointer {} at ({}, {})", action, x, y)
        }
        "input.key" => {
            let evt = params.get("event").unwrap_or(&Value::Null);
            let action = evt
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("press");
            let keysym = evt.get("keysym").and_then(|v| v.as_str()).unwrap_or("?");
            format!("send key {} '{}'", action, keysym)
        }
        "input.text" => {
            let text = params
                .get("text_input")
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = if text.len() > 40 {
                format!("{}…", &text[..40])
            } else {
                text.to_string()
            };
            format!("type {} chars: '{}'", text.len(), preview)
        }
        "process.shell" => {
            let cmd = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let preview = if cmd.len() > 80 { &cmd[..80] } else { cmd };
            format!("run shell: '{preview}'")
        }
        "process.exec" => {
            let argv = params.get("argv").and_then(|v| v.as_array());
            let joined = argv
                .map(|arr| {
                    arr.iter()
                        .map(|v| v.as_str().unwrap_or(""))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let preview = if joined.len() > 80 {
                &joined[..80]
            } else {
                &joined
            };
            format!("exec: '{preview}'")
        }
        m if m.starts_with("fs.") => {
            let verb = m.strip_prefix("fs.").unwrap_or(m);
            if verb == "move" {
                let src = params.get("src").and_then(|v| v.as_str()).unwrap_or("?");
                let dst = params.get("dst").and_then(|v| v.as_str()).unwrap_or("?");
                format!("move {} → {}", src, dst)
            } else {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                format!("{} file: {}", verb, path)
            }
        }
        "clipboard.write" => "write to the clipboard".to_string(),
        m if m.starts_with("apps.") => {
            let verb = m.strip_prefix("apps.").unwrap_or(m);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{} app: '{}'", verb, name)
        }
        m if m.starts_with("windows.") => {
            if let Some(obj) = params.as_object() {
                let pairs: Vec<String> = obj.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                format!("{} ({})", m, pairs.join(", "))
            } else {
                m.to_string()
            }
        }
        "ui.action" => {
            let elem = params.get("element").unwrap_or(&Value::Null);
            let action = params
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("click");
            let target = elem
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| elem.get("role").and_then(|v| v.as_str()))
                .unwrap_or("?");
            format!("ui action '{}' on {}", action, target)
        }
        "power.lock" => "lock the screen".to_string(),
        _ => method.to_string(),
    }
}

/// Format an audit log entry for console display.
pub fn format_audit_entry(entry: &Value) -> String {
    let ts = entry.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let dt = chrono::DateTime::from_timestamp(ts as i64, ((ts.fract()) * 1_000_000_000.0) as u32)
        .map(|d| d.naive_local().format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "1970-01-01 00:00:00".to_string());

    let peer_obj = entry.get("peer").unwrap_or(&Value::Null);
    let peer = peer_obj
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| peer_obj.get("fp").and_then(|v| v.as_str()))
        .unwrap_or("?");

    let kind = entry.get("type").and_then(|v| v.as_str()).unwrap_or("?");

    match kind {
        "call" => {
            let outcome = entry.get("outcome").and_then(|v| v.as_str()).unwrap_or("?");
            let outcome_str = if outcome == "error" {
                format!(
                    "error/{}",
                    entry
                        .get("error_code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                )
            } else {
                outcome.to_string()
            };
            let summary = entry
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| entry.get("method").and_then(|v| v.as_str()).unwrap_or("?"));

            format!("{dt}  {peer:<16}  {kind:<14}  {outcome_str:<22}  {summary}")
        }
        "session.opened" => {
            let session_id = entry
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let remote_addr = entry
                .get("remote_addr")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!(
                "{dt}  {peer:<16}  {kind:<14}                          id={session_id} from {remote_addr}"
            )
        }
        "session.closed" => {
            let session_id = entry
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let duration = entry
                .get("duration")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let reason = entry.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            format!(
                "{dt}  {peer:<16}  {kind:<14}                          id={session_id}  duration={duration}s  reason={reason:?}"
            )
        }
        "session.rejected" => {
            let reason = entry.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
            let remote_addr = entry
                .get("remote_addr")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!(
                "{dt}  {peer:<16}  {kind:<14}                          reason={reason}  from {remote_addr}"
            )
        }
        _ => format!("{dt}  {peer:<16}  {kind}  {entry}"),
    }
}
