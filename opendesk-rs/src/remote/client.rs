//! RemoteComputer and client connection helpers: pair_with and connect.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;

use super::transport::WebSocketTransport;
use crate::protocol::crypto::EncryptedChannel;
use crate::protocol::frames::{Frame, HelloFrame, ReqFrame, rmpv_to_json, value_get};
use crate::protocol::handshake::{Transport, auth_client, pair_client};
use crate::protocol::identity::Identity;
use crate::protocol::storage::{TrustedPeers, default_peer_name};

/// The connection to the peer failed while performing an RPC.
///
/// Once a `RemoteComputer` returns this error it is permanently unusable
/// (see [`RemoteComputer::is_broken`]); callers should drop it and reconnect.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// The request was never delivered to the peer, so it did not run there.
    #[error("{0:#}")]
    Send(anyhow::Error),
    /// The request was sent but no response could be read; it may or may not
    /// have run on the peer.
    #[error("{0:#}")]
    Recv(anyhow::Error),
}

pub struct RemoteComputer {
    transport: Arc<Mutex<WebSocketTransport>>,
    channel: Arc<Mutex<EncryptedChannel>>,
    capabilities: HashMap<String, rmpv::Value>,
    req_counter: AtomicU64,
    requests_sent: AtomicU64,
    broken: AtomicBool,
    last_used: std::sync::Mutex<Instant>,
    input_guard: std::sync::Mutex<Option<crate::computer::foreground::WindowGuard>>,
}

impl RemoteComputer {
    /// Attach `guard` to every subsequent `input.*` RPC: the peer refuses
    /// the input unless its foreground window matches. `None` removes it.
    pub fn set_input_guard(&self, guard: Option<crate::computer::foreground::WindowGuard>) {
        if let Ok(mut g) = self.input_guard.lock() {
            *g = guard.filter(|g| !g.is_empty());
        }
    }

    pub fn new(
        transport: WebSocketTransport,
        channel: EncryptedChannel,
        capabilities: HashMap<String, rmpv::Value>,
    ) -> Self {
        Self {
            transport: Arc::new(Mutex::new(transport)),
            channel: Arc::new(Mutex::new(channel)),
            capabilities,
            req_counter: AtomicU64::new(1),
            requests_sent: AtomicU64::new(0),
            broken: AtomicBool::new(false),
            last_used: std::sync::Mutex::new(Instant::now()),
            input_guard: std::sync::Mutex::new(None),
        }
    }

    pub fn capabilities(&self) -> &HashMap<String, rmpv::Value> {
        &self.capabilities
    }

    /// True once an RPC has failed with a [`ConnectionError`].
    pub fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    /// Number of requests successfully written to the transport so far.
    ///
    /// If this is unchanged across a failed operation, nothing reached the
    /// peer and the operation is safe to retry on a fresh connection.
    pub fn requests_sent(&self) -> u64 {
        self.requests_sent.load(Ordering::SeqCst)
    }

    /// Time since the last successful RPC (or since the connection was made).
    pub fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    /// Round-trip a cheap read-only RPC to confirm the peer is still reachable
    /// end to end. Marks the connection broken on failure or timeout; an error
    /// *reply* from the peer still counts as alive.
    pub async fn ping(&self, timeout: Duration) -> Result<()> {
        match tokio::time::timeout(timeout, self.call("system.environment", HashMap::new())).await {
            Ok(Err(e)) if e.is::<ConnectionError>() => Err(e),
            Ok(_) => Ok(()),
            Err(_) => {
                // A late response would desync the request/response stream.
                self.broken.store(true, Ordering::SeqCst);
                Err(ConnectionError::Recv(anyhow!("ping timed out after {timeout:?}")).into())
            }
        }
    }

    pub async fn call(
        &self,
        method: &str,
        mut params: HashMap<String, rmpv::Value>,
    ) -> Result<rmpv::Value> {
        if self.is_broken() {
            return Err(ConnectionError::Send(anyhow!("connection to peer is closed")).into());
        }
        if method == "input.pointer" {
            // Callers pass coordinates read off this peer's screenshots; the
            // peer maps them onto its desktop. Older peers ignore the tag.
            params
                .entry("space".to_string())
                .or_insert_with(|| rmpv::Value::from("image"));
        }
        if method.starts_with("input.") {
            let guard = self.input_guard.lock().ok().and_then(|g| g.clone());
            if let Some(guard) = guard {
                // An older peer would silently ignore the guard.
                if !self.capabilities.contains_key("input.guard") {
                    bail!(
                        "input refused: this peer's opendesk is too old to enforce a window guard ({}); update it, or call without a guard",
                        guard.describe()
                    );
                }
                params.insert("guard".to_string(), guard.to_rmpv());
            }
        }
        let id = self.req_counter.fetch_add(1, Ordering::SeqCst);
        let req = Frame::Req(ReqFrame::new(id, method, params));
        let req_bytes = req.to_msgpack()?;
        let ct = {
            let mut chan = self.channel.lock().await;
            chan.encrypt(&req_bytes)?
        };

        {
            let mut tr = self.transport.lock().await;
            if let Err(e) = tr.send(&ct).await {
                self.broken.store(true, Ordering::SeqCst);
                return Err(ConnectionError::Send(e).into());
            }
        }
        self.requests_sent.fetch_add(1, Ordering::SeqCst);

        // Await ResFrame
        loop {
            let res_ct = {
                let mut tr = self.transport.lock().await;
                match tr.recv().await {
                    Ok(ct) => ct,
                    Err(e) => {
                        self.broken.store(true, Ordering::SeqCst);
                        return Err(ConnectionError::Recv(e).into());
                    }
                }
            };

            let pt = {
                let mut chan = self.channel.lock().await;
                chan.decrypt(&res_ct)?
            };

            let frame = Frame::from_msgpack(&pt)?;
            match frame {
                Frame::Res(res) => {
                    if res.id == id {
                        if let Ok(mut t) = self.last_used.lock() {
                            *t = Instant::now();
                        }
                        if res.error.is_none() {
                            return Ok(res.result.unwrap_or(rmpv::Value::Nil));
                        } else {
                            let err_msg = res
                                .error
                                .map(|e| {
                                    let s = e.to_string();
                                    let trimmed = s.trim();
                                    if let Some(unquoted) =
                                        trimmed.strip_prefix('"').and_then(|t| t.strip_suffix('"'))
                                    {
                                        unquoted.to_string()
                                    } else {
                                        s
                                    }
                                })
                                .unwrap_or_else(|| "unknown remote error".to_string());
                            return Err(anyhow!(err_msg));
                        }
                    }
                }
                Frame::Push(_) | Frame::Hello(_) | Frame::Cancel(_) | Frame::Req(_) => {
                    continue;
                }
            }
        }
    }

    // Convenience API methods matching Python OpenDesk RemoteComputer

    pub async fn screenshot(&self, format: &str, target: Option<&str>) -> Result<String> {
        let (bytes, _mime) = self.screenshot_format(target, Some(format), None).await?;
        Ok(data_encoding::BASE64.encode(&bytes))
    }

    pub async fn screenshot_bytes(&self, target: Option<&str>) -> Result<Vec<u8>> {
        self.screenshot_format(target, None, None)
            .await
            .map(|(b, _)| b)
    }

    pub async fn screenshot_format(
        &self,
        target: Option<&str>,
        format: Option<&str>,
        quality: Option<u8>,
    ) -> Result<(Vec<u8>, String)> {
        self.screenshot_region_format(target, None, format, quality)
            .await
    }

    /// Like `screenshot_format`, limited to `region` (desktop coordinates)
    /// when given. Peers without region support return the full screen.
    pub async fn screenshot_region_format(
        &self,
        target: Option<&str>,
        region: Option<xa11y::Rect>,
        format: Option<&str>,
        quality: Option<u8>,
    ) -> Result<(Vec<u8>, String)> {
        let mut params = HashMap::new();
        if let Some(r) = region {
            params.insert(
                "region".to_string(),
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("x"), rmpv::Value::from(r.x)),
                    (rmpv::Value::from("y"), rmpv::Value::from(r.y)),
                    (rmpv::Value::from("width"), rmpv::Value::from(r.width)),
                    (rmpv::Value::from("height"), rmpv::Value::from(r.height)),
                ]),
            );
        }
        if let Some(t) = target {
            params.insert("display_id".to_string(), rmpv::Value::from(t));
        }
        if let Some(f) = format {
            params.insert("format".to_string(), rmpv::Value::from(f));
        }
        if let Some(q) = quality {
            params.insert("quality".to_string(), rmpv::Value::from(q as i64));
        }
        let res = self.call("display.capture", params).await?;
        let mime = value_get(&res, "format")
            .and_then(|v| v.as_str())
            .map(|f| {
                if f.contains('/') {
                    f.to_string()
                } else if f == "jpeg" || f == "jpg" {
                    "image/jpeg".to_string()
                } else {
                    "image/png".to_string()
                }
            })
            .unwrap_or_else(|| "image/png".to_string());

        if let Some(data_val) = value_get(&res, "data") {
            if let Some(bytes) = data_val.as_slice() {
                return Ok((bytes.to_vec(), mime));
            }
            if let Some(decoded) = data_val
                .as_str()
                .and_then(|s| data_encoding::BASE64.decode(s.as_bytes()).ok())
            {
                return Ok((decoded, mime));
            }
        }
        bail!("missing binary image data in display.capture response")
    }

    pub async fn mouse_move(&self, x: i32, y: i32) -> Result<()> {
        let evt = vec![
            (rmpv::Value::from("action"), rmpv::Value::from("move")),
            (
                rmpv::Value::from("point"),
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("x"), rmpv::Value::from(x)),
                    (rmpv::Value::from("y"), rmpv::Value::from(y)),
                ]),
            ),
        ];
        let mut params = HashMap::new();
        params.insert("event".to_string(), rmpv::Value::Map(evt));
        self.call("input.pointer", params).await?;
        Ok(())
    }

    pub async fn mouse_click(&self, x: i32, y: i32, button: Option<&str>) -> Result<()> {
        let btn_str = match button {
            Some("right") => "right",
            Some("middle") => "middle",
            _ => "left",
        };

        // 1. Move to position
        self.mouse_move(x, y).await?;

        // 2. Down
        let down_evt = vec![
            (rmpv::Value::from("action"), rmpv::Value::from("down")),
            (
                rmpv::Value::from("point"),
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("x"), rmpv::Value::from(x)),
                    (rmpv::Value::from("y"), rmpv::Value::from(y)),
                ]),
            ),
            (rmpv::Value::from("button"), rmpv::Value::from(btn_str)),
        ];
        let mut down_params = HashMap::new();
        down_params.insert("event".to_string(), rmpv::Value::Map(down_evt));
        self.call("input.pointer", down_params).await?;

        // 3. Up
        let up_evt = vec![
            (rmpv::Value::from("action"), rmpv::Value::from("up")),
            (
                rmpv::Value::from("point"),
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("x"), rmpv::Value::from(x)),
                    (rmpv::Value::from("y"), rmpv::Value::from(y)),
                ]),
            ),
            (rmpv::Value::from("button"), rmpv::Value::from(btn_str)),
        ];
        let mut up_params = HashMap::new();
        up_params.insert("event".to_string(), rmpv::Value::Map(up_evt));
        self.call("input.pointer", up_params).await?;
        Ok(())
    }

    pub async fn mouse_double_click(&self, x: i32, y: i32) -> Result<()> {
        self.mouse_click(x, y, None).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        self.mouse_click(x, y, None).await?;
        Ok(())
    }

    pub async fn mouse_drag(
        &self,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<()> {
        self.mouse_move(start_x, start_y).await?;

        let down_evt = vec![
            (rmpv::Value::from("action"), rmpv::Value::from("down")),
            (
                rmpv::Value::from("point"),
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("x"), rmpv::Value::from(start_x)),
                    (rmpv::Value::from("y"), rmpv::Value::from(start_y)),
                ]),
            ),
            (rmpv::Value::from("button"), rmpv::Value::from("left")),
        ];
        let mut down_params = HashMap::new();
        down_params.insert("event".to_string(), rmpv::Value::Map(down_evt));
        self.call("input.pointer", down_params).await?;

        self.mouse_move(end_x, end_y).await?;

        let up_evt = vec![
            (rmpv::Value::from("action"), rmpv::Value::from("up")),
            (
                rmpv::Value::from("point"),
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("x"), rmpv::Value::from(end_x)),
                    (rmpv::Value::from("y"), rmpv::Value::from(end_y)),
                ]),
            ),
            (rmpv::Value::from("button"), rmpv::Value::from("left")),
        ];
        let mut up_params = HashMap::new();
        up_params.insert("event".to_string(), rmpv::Value::Map(up_evt));
        self.call("input.pointer", up_params).await?;
        Ok(())
    }

    pub async fn mouse_scroll(&self, x: i32, y: i32, delta_y: i32) -> Result<()> {
        self.mouse_scroll_xy(x, y, 0, delta_y).await
    }

    /// Scroll by ticks: positive `delta_y` up, positive `delta_x` right.
    pub async fn mouse_scroll_xy(&self, x: i32, y: i32, delta_x: i32, delta_y: i32) -> Result<()> {
        let evt = vec![
            (rmpv::Value::from("action"), rmpv::Value::from("scroll")),
            (
                rmpv::Value::from("point"),
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("x"), rmpv::Value::from(x)),
                    (rmpv::Value::from("y"), rmpv::Value::from(y)),
                ]),
            ),
            (rmpv::Value::from("dx"), rmpv::Value::from(delta_x as f64)),
            (rmpv::Value::from("dy"), rmpv::Value::from(delta_y as f64)),
        ];
        let mut params = HashMap::new();
        params.insert("event".to_string(), rmpv::Value::Map(evt));
        self.call("input.pointer", params).await?;
        Ok(())
    }

    pub async fn keyboard_type(&self, text: &str) -> Result<()> {
        let ti = vec![
            (rmpv::Value::from("text"), rmpv::Value::from(text)),
            (rmpv::Value::from("interval_ms"), rmpv::Value::from(10)),
        ];
        let mut params = HashMap::new();
        params.insert("text_input".to_string(), rmpv::Value::Map(ti));
        self.call("input.text", params).await?;
        Ok(())
    }

    /// Press a key or chord (`ctrl+s`, `ctrl+-`) on the peer in one RPC.
    ///
    /// The peer presses and releases it locally: sending separate down/up
    /// RPCs would leave keys held if the connection dropped in between, and
    /// would bypass the peer's layout-aware handling of characters.
    pub async fn keyboard_press(&self, key: &str) -> Result<()> {
        if crate::computer::keys::split_chord(key).is_empty() {
            bail!("empty key");
        }
        let mut params = HashMap::new();
        params.insert("key".to_string(), rmpv::Value::from(key));
        params.insert("action".to_string(), rmpv::Value::from("press"));
        self.call("input.key", params).await?;
        Ok(())
    }

    pub async fn keyboard_hotkey(&self, keys: &[&str]) -> Result<()> {
        let flat: Vec<String> = keys
            .iter()
            .flat_map(|k| crate::computer::keys::split_chord(k))
            .collect();
        if flat.is_empty() {
            bail!("empty key combination");
        }
        // `split_chord` reads a `+` in key position as the key, so joining
        // with `+` round-trips (["ctrl", "+"] -> "ctrl++").
        self.keyboard_press(&flat.join("+")).await
    }

    pub async fn keyboard_sequence(
        &self,
        steps: &[crate::computer::local::KeyboardStep],
    ) -> Result<()> {
        let mut steps_val = Vec::with_capacity(steps.len());
        for s in steps {
            let mut map = Vec::new();
            if let Some(ref a) = s.action {
                map.push((rmpv::Value::from("action"), rmpv::Value::from(a.as_str())));
            }
            if let Some(ref k) = s.key {
                map.push((rmpv::Value::from("key"), rmpv::Value::from(k.as_str())));
            }
            if let Some(ref t) = s.text {
                map.push((rmpv::Value::from("text"), rmpv::Value::from(t.as_str())));
            }
            if let Some(ref d) = s.delay_ms {
                map.push((rmpv::Value::from("delay_ms"), rmpv::Value::from(*d as i64)));
            }
            if let Some(ref ks) = s.keys {
                let arr = ks
                    .iter()
                    .map(|item| rmpv::Value::from(item.as_str()))
                    .collect();
                map.push((rmpv::Value::from("keys"), rmpv::Value::Array(arr)));
            }
            if let Some(ref hk) = s.hotkey {
                match hk {
                    serde_json::Value::Array(arr) => {
                        let rarr = arr
                            .iter()
                            .filter_map(|v| v.as_str())
                            .map(rmpv::Value::from)
                            .collect();
                        map.push((rmpv::Value::from("hotkey"), rmpv::Value::Array(rarr)));
                    }
                    serde_json::Value::String(str_val) => {
                        map.push((
                            rmpv::Value::from("hotkey"),
                            rmpv::Value::from(str_val.as_str()),
                        ));
                    }
                    _ => {}
                }
            }
            steps_val.push(rmpv::Value::Map(map));
        }
        let mut params = HashMap::new();
        params.insert("steps".to_string(), rmpv::Value::Array(steps_val));
        self.call("input.sequence", params).await?;
        Ok(())
    }

    pub async fn exec(
        &self,
        command: &str,
        shell: Option<&str>,
        timeout_secs: Option<u64>,
    ) -> Result<crate::computer::local::ExecResult> {
        let mut params = HashMap::new();
        params.insert("command".to_string(), rmpv::Value::from(command));
        if let Some(s) = shell {
            params.insert("shell".to_string(), rmpv::Value::from(s));
        }
        if let Some(t) = timeout_secs {
            params.insert("timeout_secs".to_string(), rmpv::Value::from(t as i64));
        }
        let res = self.call("process.shell", params).await?;
        let stdout = value_get(&res, "stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let stderr = value_get(&res, "stderr")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let exit_code = value_get(&res, "exit_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let success = value_get(&res, "success")
            .and_then(|v| v.as_bool())
            .unwrap_or(exit_code == 0);
        Ok(crate::computer::local::ExecResult {
            stdout,
            stderr,
            exit_code,
            success,
            timed_out: value_get(&res, "timed_out")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    pub async fn exec_argv(
        &self,
        argv: &[&str],
        timeout_secs: Option<u64>,
    ) -> Result<crate::computer::local::ExecResult> {
        let mut params = HashMap::new();
        let arr = argv.iter().map(|s| rmpv::Value::from(*s)).collect();
        params.insert("argv".to_string(), rmpv::Value::Array(arr));
        if let Some(t) = timeout_secs {
            params.insert("timeout_secs".to_string(), rmpv::Value::from(t as i64));
        }
        let res = self.call("process.exec", params).await?;
        let stdout = value_get(&res, "stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let stderr = value_get(&res, "stderr")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let exit_code = value_get(&res, "exit_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let success = value_get(&res, "success")
            .and_then(|v| v.as_bool())
            .unwrap_or(exit_code == 0);
        Ok(crate::computer::local::ExecResult {
            stdout,
            stderr,
            exit_code,
            success,
            timed_out: value_get(&res, "timed_out")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    pub async fn app_open(&self, path: &str) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("name".to_string(), rmpv::Value::from(path));
        self.call("apps.open", params).await?;
        Ok(())
    }

    pub async fn app_focus(&self, name: &str) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("name".to_string(), rmpv::Value::from(name));
        self.call("apps.focus", params).await?;
        Ok(())
    }

    pub async fn app_close(&self, name: &str) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("name".to_string(), rmpv::Value::from(name));
        self.call("apps.close", params).await?;
        Ok(())
    }

    pub async fn app_list(&self) -> Result<Vec<Value>> {
        let res = self.call("apps.list", HashMap::new()).await?;
        if let Some(items) = value_get(&res, "items").and_then(|v| v.as_array()) {
            return Ok(items
                .iter()
                .map(|item| {
                    if let Some(s) = item.as_str() {
                        json!({ "name": s, "pid": Value::Null })
                    } else {
                        rmpv_to_json(item)
                    }
                })
                .collect());
        }
        bail!("missing 'items' in apps.list response")
    }

    pub async fn ui_tree(
        &self,
        app_name: Option<&str>,
        max_depth: Option<usize>,
    ) -> Result<String> {
        let mut params = HashMap::new();
        if let Some(app) = app_name {
            params.insert("app".to_string(), rmpv::Value::from(app));
        }
        if let Some(d) = max_depth {
            params.insert("max_depth".to_string(), rmpv::Value::from(d as u64));
        }
        let res = self.call("ui.tree", params).await?;
        if value_get(&res, "role").is_some() {
            let mut out = String::new();
            format_rmpv_ui_element(&res, 0, &mut out);
            return Ok(out);
        }
        if let Some(tree) = value_get(&res, "tree").and_then(|v| v.as_str()) {
            return Ok(tree.to_string());
        }
        Ok(rmpv_to_json(&res).to_string())
    }

    pub async fn ui_click(&self, app_name: Option<&str>, selector: &str) -> Result<()> {
        let elem = vec![
            (rmpv::Value::from("role"), rmpv::Value::from("")),
            (rmpv::Value::from("name"), rmpv::Value::from(selector)),
        ];
        let mut params = HashMap::new();
        params.insert("element".to_string(), rmpv::Value::Map(elem));
        params.insert("action".to_string(), rmpv::Value::from("click"));
        if let Some(app) = app_name {
            params.insert("app".to_string(), rmpv::Value::from(app));
        }
        self.call("ui.action", params).await?;
        Ok(())
    }

    /// The peer's environment, and how far its clock is ahead of ours (ms,
    /// negative = behind), estimated at the midpoint of the round trip.
    /// The offset is `None` for older peers that do not report their clock.
    pub async fn environment(
        &self,
    ) -> Result<(crate::computer::environment::Environment, Option<i64>)> {
        let t0 = chrono::Utc::now().timestamp_millis();
        let res = self.call("system.environment", HashMap::new()).await?;
        let t1 = chrono::Utc::now().timestamp_millis();
        let env: crate::computer::environment::Environment = rmpv::ext::from_value(res)
            .map_err(|e| anyhow!("malformed system.environment reply: {e}"))?;
        let offset = (env.time_unix_ms > 0).then(|| env.time_unix_ms - (t0 + t1) / 2);
        Ok((env, offset))
    }

    fn require_cap(&self, cap: &str, what: &str) -> Result<()> {
        if self.capabilities.contains_key(cap) {
            Ok(())
        } else {
            bail!("this peer's opendesk is too old for {what} ({cap}); update it")
        }
    }

    /// Wait on the peer until `selector` reaches `state` (see computer::watch).
    pub async fn ui_wait(
        &self,
        app_name: Option<&str>,
        selector: &str,
        state: &str,
        timeout: Duration,
    ) -> Result<String> {
        self.require_cap("ui.wait", "waiting on UI state")?;
        let mut params = HashMap::new();
        params.insert("selector".to_string(), rmpv::Value::from(selector));
        params.insert("state".to_string(), rmpv::Value::from(state));
        params.insert(
            "timeout_ms".to_string(),
            rmpv::Value::from(timeout.as_millis() as u64),
        );
        if let Some(app) = app_name {
            params.insert("app".to_string(), rmpv::Value::from(app));
        }
        let res = self.call("ui.wait", params).await?;
        Ok(res.as_str().unwrap_or_default().to_string())
    }

    pub async fn events_start(&self, app_name: Option<&str>) -> Result<String> {
        self.require_cap("events", "event logs")?;
        let mut params = HashMap::new();
        if let Some(app) = app_name {
            params.insert("app".to_string(), rmpv::Value::from(app));
        }
        let res = self.call("events.start", params).await?;
        Ok(res.as_str().unwrap_or_default().to_string())
    }

    pub async fn events_read(&self, wait: Duration) -> Result<crate::computer::watch::EventBatch> {
        self.require_cap("events", "event logs")?;
        let mut params = HashMap::new();
        params.insert(
            "wait_ms".to_string(),
            rmpv::Value::from(wait.as_millis() as u64),
        );
        let res = self.call("events.read", params).await?;
        rmpv::ext::from_value(res).map_err(|e| anyhow!("malformed events.read reply: {e}"))
    }

    pub async fn events_stop(&self) -> Result<bool> {
        self.require_cap("events", "event logs")?;
        let res = self.call("events.stop", HashMap::new()).await?;
        Ok(res.as_bool().unwrap_or(false))
    }

    /// Value (or name) of the element matching `selector` on the peer.
    pub async fn ui_get_value(&self, app_name: Option<&str>, selector: &str) -> Result<String> {
        if !self.capabilities.contains_key("ui.value") {
            bail!("this peer's opendesk is too old to read element values (ui.value); update it");
        }
        let mut params = HashMap::new();
        params.insert("selector".to_string(), rmpv::Value::from(selector));
        if let Some(app) = app_name {
            params.insert("app".to_string(), rmpv::Value::from(app));
        }
        let res = self.call("ui.value", params).await?;
        Ok(res.as_str().unwrap_or_default().to_string())
    }

    pub async fn ui_type(&self, app_name: Option<&str>, selector: &str, text: &str) -> Result<()> {
        self.ui_click(app_name, selector).await?;
        self.keyboard_type(text).await?;
        Ok(())
    }

    pub async fn clipboard_read(&self) -> Result<String> {
        let res = self.call("clipboard.read", HashMap::new()).await?;
        if let Some(entries) = value_get(&res, "entries").and_then(|v| v.as_array()) {
            for entry in entries {
                if let Some(data) = value_get(entry, "data") {
                    if let Some(text) = data.as_slice().and_then(|b| std::str::from_utf8(b).ok()) {
                        return Ok(text.to_string());
                    }
                    if let Some(s) = data.as_str() {
                        return Ok(s.to_string());
                    }
                }
            }
        }
        if let Some(text) = value_get(&res, "text").and_then(|v| v.as_str()) {
            return Ok(text.to_string());
        }
        bail!("no text found in clipboard response")
    }

    pub async fn clipboard_write(&self, text: &str) -> Result<()> {
        let entry = vec![
            (
                rmpv::Value::from("mime_type"),
                rmpv::Value::from("text/plain;charset=utf-8"),
            ),
            (
                rmpv::Value::from("data"),
                rmpv::Value::Binary(text.as_bytes().to_vec()),
            ),
        ];
        let contents = rmpv::Value::Map(vec![(
            rmpv::Value::from("entries"),
            rmpv::Value::Array(vec![rmpv::Value::Map(entry)]),
        )]);
        let mut params = HashMap::new();
        params.insert("contents".to_string(), contents);
        self.call("clipboard.write", params).await?;
        Ok(())
    }

    pub async fn set_privacy(
        &self,
        lock_input: bool,
        blackout: bool,
    ) -> Result<crate::computer::privacy::PrivacyState> {
        let mut params = HashMap::new();
        params.insert("lock_input".to_string(), rmpv::Value::Boolean(lock_input));
        params.insert("blackout".to_string(), rmpv::Value::Boolean(blackout));
        let res = self.call("system.privacy", params).await?;
        let lock_input = value_get(&res, "lock_input")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let blackout = value_get(&res, "blackout")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let supported = value_get(&res, "supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(crate::computer::privacy::PrivacyState {
            lock_input,
            blackout,
            supported,
        })
    }

    pub async fn get_privacy(&self) -> Result<crate::computer::privacy::PrivacyState> {
        let res = self.call("system.privacy_status", HashMap::new()).await?;
        let lock_input = value_get(&res, "lock_input")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let blackout = value_get(&res, "blackout")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let supported = value_get(&res, "supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(crate::computer::privacy::PrivacyState {
            lock_input,
            blackout,
            supported,
        })
    }
}

fn format_rmpv_ui_element(el: &rmpv::Value, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let role = value_get(el, "role")
        .and_then(|v| v.as_str())
        .unwrap_or("element");
    let name = value_get(el, "name").and_then(|v| v.as_str()).unwrap_or("");
    let value = value_get(el, "value").and_then(|v| v.as_str());

    out.push_str(&indent);
    out.push_str(role);
    if !name.is_empty() {
        out.push_str(&format!(" name=\"{}\"", name));
    }
    if let Some(v) = value
        && !v.is_empty()
    {
        let escaped = v
            .replace('\\', "\\\\")
            .replace('\"', "\\\"")
            .replace("\r\n", "\\n")
            .replace(['\r', '\n'], "\\n");
        out.push_str(&format!(" value=\"{}\"", escaped));
    }
    out.push('\n');

    if let Some(children) = value_get(el, "children").and_then(|v| v.as_array()) {
        for child in children {
            format_rmpv_ui_element(child, depth + 1, out);
        }
    }
}

/// Pair with a remote peer running `opendesk pair`.
#[allow(clippy::too_many_arguments)]
pub async fn pair_with(
    host: Option<&str>,
    port: Option<u16>,
    code: &str,
    name: Option<&str>,
    rendezvous_url: Option<&str>,
    rendezvous_token: Option<&str>,
    target_pubkey: Option<&str>,
    home: Option<&Path>,
) -> Result<(RemoteComputer, [u8; 32])> {
    let identity = Identity::load_or_create(home)?;
    let trusted = TrustedPeers::new(home);

    let mut resolved_endpoint = ("127.0.0.1".to_string(), 8423u16);
    let (mut transport, _server_pubkey) = if let Some(r_url) = rendezvous_url {
        let target_pk = if let Some(tp) = target_pubkey {
            tp.to_string()
        } else if let Some(h) = host {
            let h_clean = h.strip_prefix("peer-").unwrap_or(h);
            if h_clean.len() == 64 && data_encoding::HEXLOWER.decode(h_clean.as_bytes()).is_ok() {
                h_clean.to_string()
            } else {
                let r_client = super::rendezvous::RendezvousClient::new(r_url, rendezvous_token);
                let peers = r_client.list_peers(Duration::from_secs(3)).await?;
                let matched = peers.into_iter().find(|p| {
                    let pk_hex = data_encoding::HEXLOWER.encode(&p.public_key);
                    p.name.eq_ignore_ascii_case(h)
                        || pk_hex.eq_ignore_ascii_case(h_clean)
                        || pk_hex.starts_with(h_clean)
                });
                match matched {
                    Some(p) => data_encoding::HEXLOWER.encode(&p.public_key),
                    None => {
                        return Err(anyhow!(
                            "no peer found on rendezvous matching '{}'. Specify --target-pubkey",
                            h
                        ));
                    }
                }
            }
        } else {
            let r_client = super::rendezvous::RendezvousClient::new(r_url, rendezvous_token);
            let peers = r_client.list_peers(Duration::from_secs(3)).await?;
            if peers.len() == 1 {
                data_encoding::HEXLOWER.encode(&peers[0].public_key)
            } else if peers.is_empty() {
                return Err(anyhow!(
                    "no peers currently online on rendezvous server. Make sure the host is running 'opendesk pair --rendezvous {}'",
                    r_url
                ));
            } else {
                let names: Vec<String> = peers
                    .into_iter()
                    .map(|p| format!("{} ({})", p.name, p.fingerprint))
                    .collect();
                return Err(anyhow!(
                    "multiple peers online on rendezvous server ({}). Please specify the peer name or --target-pubkey",
                    names.join(", ")
                ));
            }
        };

        let pk_bytes = data_encoding::HEXLOWER
            .decode(target_pk.as_bytes())
            .or_else(|_| data_encoding::HEXUPPER.decode(target_pk.as_bytes()))
            .with_context(|| format!("invalid target_pubkey hex: {}", target_pk))?;
        if pk_bytes.len() != 32 {
            return Err(anyhow!("target_pubkey must be 32 bytes"));
        }
        let mut target_arr = [0u8; 32];
        target_arr.copy_from_slice(&pk_bytes);

        let tr =
            super::rendezvous::connect_via_rendezvous(r_url, rendezvous_token, &target_arr).await?;
        (tr, target_arr)
    } else {
        let host_input = host.ok_or_else(|| anyhow!("host is required for direct pairing"))?;
        let (raw_host, parsed_port) = if let Some((h, p)) = host_input.split_once(':') {
            (h, p.parse::<u16>().ok())
        } else {
            (host_input, None)
        };
        let target_port = parsed_port.or(port).unwrap_or(8423);

        let target_host = if raw_host.parse::<std::net::IpAddr>().is_err() {
            // If raw_host is not an IP address (e.g. "old-pc"), try resolving via mDNS discovery
            if let Ok(peers) = super::discovery::discover(Duration::from_secs(2)).await {
                if let Some(p) = peers
                    .into_iter()
                    .find(|p| p.name.eq_ignore_ascii_case(raw_host))
                {
                    p.host
                } else {
                    raw_host.to_string()
                }
            } else {
                raw_host.to_string()
            }
        } else {
            raw_host.to_string()
        };

        let ws_url = format!("ws://{}:{}", target_host, target_port);
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .with_context(|| format!("failed to connect to {}", ws_url))?;
        resolved_endpoint = (target_host, target_port);
        (WebSocketTransport::new_tls(ws_stream), [0u8; 32])
    };

    // Run client pairing
    let session = pair_client(&mut transport, &identity, code).await?;
    let verified_pubkey = session.peer_public;

    let peer_name = name
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| default_peer_name(&verified_pubkey));

    if let Some(r_url) = rendezvous_url {
        trusted.add(&verified_pubkey, &peer_name, r_url)?;
    } else {
        trusted.add(&verified_pubkey, &peer_name, "")?;
        trusted.cache_endpoint(&verified_pubkey, &resolved_endpoint.0, resolved_endpoint.1)?;
    }

    // Exchange Hello
    let mut channel = session.channel;
    let hello = Frame::Hello(HelloFrame::client());
    let hello_bytes = hello.to_msgpack()?;
    let ct = channel.encrypt(&hello_bytes)?;
    transport.send(&ct).await?;

    let server_hello_ct = transport.recv().await?;
    let server_hello_pt = channel.decrypt(&server_hello_ct)?;
    let server_frame = Frame::from_msgpack(&server_hello_pt)?;

    let caps = match server_frame {
        Frame::Hello(h) => {
            if let Some(desc_val) = h.capabilities.get("description")
                && let Some(desc) = desc_val.as_str()
            {
                let _ = trusted.cache_description(&verified_pubkey, desc);
            }
            h.capabilities
        }
        _ => HashMap::new(),
    };

    let remote = RemoteComputer::new(transport, channel, caps);
    Ok((remote, verified_pubkey))
}

/// Connect to a paired peer and return a RemoteComputer.
pub async fn connect(
    peer_query: Option<&str>,
    rendezvous_url: Option<&str>,
    rendezvous_token: Option<&str>,
    home: Option<&Path>,
) -> Result<RemoteComputer> {
    let identity = Identity::load_or_create(home)?;
    let trusted = TrustedPeers::new(home);

    let target_name = match peer_query {
        Some(q) => q.to_string(),
        None => trusted
            .get_default()
            .ok_or_else(|| anyhow!("no peer specified and no default peer set"))?,
    };

    let peer = match trusted.find_by_name_or_key(&target_name) {
        Some(p) => p,
        None => {
            if let Some(r_url) = rendezvous_url {
                let r_client = super::rendezvous::RendezvousClient::new(r_url, rendezvous_token);
                if let Ok(peers) = r_client.list_peers(Duration::from_secs(3)).await {
                    let t_clean = target_name.strip_prefix("peer-").unwrap_or(&target_name);
                    let matched = peers.into_iter().find(|p| {
                        let pk_hex = data_encoding::HEXLOWER.encode(&p.public_key);
                        p.name.eq_ignore_ascii_case(&target_name)
                            || pk_hex.eq_ignore_ascii_case(t_clean)
                            || pk_hex.starts_with(t_clean)
                    });
                    if let Some(mp) = matched {
                        if mp.public_key.len() == 32 {
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(&mp.public_key);
                            if let Some(tp) = trusted.find(&arr) {
                                tp
                            } else {
                                return Err(anyhow!(
                                    "peer '{}' ({}) is online on rendezvous relay, but not yet paired with this machine. Run 'opendesk pair-with ...' first.",
                                    mp.name,
                                    mp.fingerprint
                                ));
                            }
                        } else {
                            return Err(anyhow!(
                                "no trusted peer found matching '{}'",
                                target_name
                            ));
                        }
                    } else {
                        return Err(anyhow!("no trusted peer found matching '{}'", target_name));
                    }
                } else {
                    return Err(anyhow!("no trusted peer found matching '{}'", target_name));
                }
            } else {
                return Err(anyhow!("no trusted peer found matching '{}'", target_name));
            }
        }
    };

    let peer_pub = peer.public_bytes()?;

    let (mut transport, mut channel) = if let Some(r_url) = rendezvous_url.or({
        if !peer.rendezvous_url.is_empty() {
            Some(peer.rendezvous_url.as_str())
        } else {
            None
        }
    }) {
        let mut tr =
            super::rendezvous::connect_via_rendezvous(r_url, rendezvous_token, &peer_pub).await?;
        let session = auth_client(&mut tr, &identity, &peer_pub).await?;
        (tr, session.channel)
    } else {
        let host = if !peer.last_host.is_empty() {
            peer.last_host.clone()
        } else {
            "127.0.0.1".to_string()
        };
        let port = if peer.last_port != 0 {
            peer.last_port
        } else {
            8423
        };
        let ws_url = format!("ws://{}:{}", host, port);
        let ws_stream = match connect_async(&ws_url).await {
            Ok((ws, _)) => ws,
            Err(initial_err) => {
                // If last_host failed, attempt LAN discovery by public key or name
                let discovered =
                    if let Ok(peers) = super::discovery::discover(Duration::from_secs(2)).await {
                        peers.into_iter().find(|p| {
                            p.public_key == peer_pub || p.name.eq_ignore_ascii_case(&target_name)
                        })
                    } else {
                        None
                    };

                if let Some(disc) = discovered {
                    let fallback_url = format!("ws://{}:{}", disc.host, disc.port);
                    let (ws, _) = connect_async(&fallback_url).await.with_context(|| {
                        format!(
                            "failed to connect to {} (mDNS fallback for {})",
                            fallback_url, ws_url
                        )
                    })?;
                    let _ = trusted.cache_endpoint(&peer_pub, &disc.host, disc.port);
                    ws
                } else {
                    return Err(anyhow!("failed to connect to {}: {}", ws_url, initial_err));
                }
            }
        };
        let mut tr = WebSocketTransport::new_tls(ws_stream);
        let session = auth_client(&mut tr, &identity, &peer_pub).await?;
        (tr, session.channel)
    };

    // Exchange Hello
    let hello = Frame::Hello(HelloFrame::client());
    let hello_bytes = hello.to_msgpack()?;
    let ct = channel.encrypt(&hello_bytes)?;
    transport.send(&ct).await?;

    let server_hello_ct = transport.recv().await?;
    let server_hello_pt = channel.decrypt(&server_hello_ct)?;
    let server_frame = Frame::from_msgpack(&server_hello_pt)?;

    let caps = match server_frame {
        Frame::Hello(h) => {
            if let Some(desc_val) = h.capabilities.get("description")
                && let Some(desc) = desc_val.as_str()
            {
                let _ = trusted.cache_description(&peer_pub, desc);
            }
            h.capabilities
        }
        _ => HashMap::new(),
    };

    Ok(RemoteComputer::new(transport, channel, caps))
}
