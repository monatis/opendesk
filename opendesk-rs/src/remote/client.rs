//! RemoteComputer and client connection helpers: pair_with and connect.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;

use super::transport::WebSocketTransport;
use crate::protocol::crypto::EncryptedChannel;
use crate::protocol::frames::{Frame, HelloFrame, ReqFrame};
use crate::protocol::handshake::{Transport, auth_client, pair_client};
use crate::protocol::identity::Identity;
use crate::protocol::storage::{TrustedPeers, default_peer_name};

pub struct RemoteComputer {
    transport: Arc<Mutex<WebSocketTransport>>,
    channel: Arc<Mutex<EncryptedChannel>>,
    capabilities: HashMap<String, Value>,
    req_counter: AtomicU64,
}

impl RemoteComputer {
    pub fn new(
        transport: WebSocketTransport,
        channel: EncryptedChannel,
        capabilities: HashMap<String, Value>,
    ) -> Self {
        Self {
            transport: Arc::new(Mutex::new(transport)),
            channel: Arc::new(Mutex::new(channel)),
            capabilities,
            req_counter: AtomicU64::new(1),
        }
    }

    pub fn capabilities(&self) -> &HashMap<String, Value> {
        &self.capabilities
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.req_counter.fetch_add(1, Ordering::SeqCst);
        let params_map: HashMap<String, Value> = match params {
            Value::Object(m) => m.into_iter().collect(),
            _ => HashMap::new(),
        };

        let req = Frame::Req(ReqFrame::new(id, method, params_map));
        let req_bytes = req.to_msgpack()?;
        let ct = {
            let mut chan = self.channel.lock().await;
            chan.encrypt(&req_bytes)?
        };

        {
            let mut tr = self.transport.lock().await;
            tr.send(&ct).await?;
        }

        // Await ResFrame
        loop {
            let res_ct = {
                let mut tr = self.transport.lock().await;
                tr.recv().await?
            };

            let pt = {
                let mut chan = self.channel.lock().await;
                chan.decrypt(&res_ct)?
            };

            let frame = Frame::from_msgpack(&pt)?;
            match frame {
                Frame::Res(res) => {
                    if res.id == id {
                        if res.error.is_none() {
                            return Ok(res.result.unwrap_or(Value::Null));
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
                    // Ignore non-matching frames or push events
                    continue;
                }
            }
        }
    }

    // Convenience API methods matching LocalComputer
    pub async fn screenshot(&self, format: &str, target: Option<&str>) -> Result<String> {
        let params = json!({
            "format": format,
            "target": target,
        });
        let res = match self.call("computer.screenshot", params.clone()).await {
            Ok(r) => r,
            Err(_) => {
                let capture_params = json!({
                    "display_id": target,
                    "downscale": false,
                });
                self.call("display.capture", capture_params).await?
            }
        };
        res.get("image_base64")
            .or_else(|| res.get("data"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("missing image data in response"))
    }

    pub async fn mouse_move(&self, x: i32, y: i32) -> Result<()> {
        match self
            .call("computer.mouse_move", json!({ "x": x, "y": y }))
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call(
                    "input.pointer",
                    json!({
                        "event": {
                            "action": "move",
                            "point": { "x": x, "y": y }
                        }
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn mouse_click(&self, x: i32, y: i32, button: Option<&str>) -> Result<()> {
        let btn_str = match button {
            Some("right") => "right",
            Some("middle") => "middle",
            _ => "left",
        };
        match self
            .call(
                "computer.mouse_click",
                json!({ "x": x, "y": y, "button": button }),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = self
                    .call(
                        "input.pointer",
                        json!({
                            "event": {
                                "action": "move",
                                "point": { "x": x, "y": y }
                            }
                        }),
                    )
                    .await;
                self.call(
                    "input.pointer",
                    json!({
                        "event": {
                            "action": "down",
                            "point": { "x": x, "y": y },
                            "button": btn_str
                        }
                    }),
                )
                .await?;
                self.call(
                    "input.pointer",
                    json!({
                        "event": {
                            "action": "up",
                            "point": { "x": x, "y": y },
                            "button": btn_str
                        }
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn mouse_double_click(&self, x: i32, y: i32) -> Result<()> {
        match self
            .call("computer.mouse_double_click", json!({ "x": x, "y": y }))
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                self.mouse_click(x, y, None).await?;
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                self.mouse_click(x, y, None).await?;
                Ok(())
            }
        }
    }

    pub async fn mouse_drag(
        &self,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<()> {
        match self
            .call(
                "computer.mouse_drag",
                json!({
                    "start_x": start_x,
                    "start_y": start_y,
                    "end_x": end_x,
                    "end_y": end_y,
                }),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = self
                    .call(
                        "input.pointer",
                        json!({
                            "event": {
                                "action": "move",
                                "point": { "x": start_x, "y": start_y }
                            }
                        }),
                    )
                    .await;
                self.call(
                    "input.pointer",
                    json!({
                        "event": {
                            "action": "down",
                            "point": { "x": start_x, "y": start_y },
                            "button": "left"
                        }
                    }),
                )
                .await?;
                self.call(
                    "input.pointer",
                    json!({
                        "event": {
                            "action": "move",
                            "point": { "x": end_x, "y": end_y }
                        }
                    }),
                )
                .await?;
                self.call(
                    "input.pointer",
                    json!({
                        "event": {
                            "action": "up",
                            "point": { "x": end_x, "y": end_y },
                            "button": "left"
                        }
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn mouse_scroll(&self, x: i32, y: i32, delta_y: i32) -> Result<()> {
        match self
            .call(
                "computer.mouse_scroll",
                json!({
                    "x": x,
                    "y": y,
                    "delta_y": delta_y,
                }),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call(
                    "input.pointer",
                    json!({
                        "event": {
                            "action": "scroll",
                            "point": { "x": x, "y": y },
                            "dx": 0.0,
                            "dy": delta_y as f64
                        }
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn keyboard_type(&self, text: &str) -> Result<()> {
        match self
            .call("computer.keyboard_type", json!({ "text": text }))
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call(
                    "input.text",
                    json!({
                        "text_input": {
                            "text": text,
                            "interval_ms": 10
                        }
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn keyboard_press(&self, key: &str) -> Result<()> {
        match self
            .call("computer.keyboard_press", json!({ "key": key }))
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call(
                    "input.key",
                    json!({
                        "event": {
                            "action": "down",
                            "keysym": key
                        }
                    }),
                )
                .await?;
                self.call(
                    "input.key",
                    json!({
                        "event": {
                            "action": "up",
                            "keysym": key
                        }
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn keyboard_hotkey(&self, keys: &[&str]) -> Result<()> {
        match self
            .call("computer.keyboard_hotkey", json!({ "keys": keys }))
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                for key in keys {
                    let _ = self
                        .call(
                            "input.key",
                            json!({
                                "event": {
                                    "action": "down",
                                    "keysym": key
                                }
                            }),
                        )
                        .await;
                }
                for key in keys.iter().rev() {
                    let _ = self
                        .call(
                            "input.key",
                            json!({
                                "event": {
                                    "action": "up",
                                    "keysym": key
                                }
                            }),
                        )
                        .await;
                }
                Ok(())
            }
        }
    }

    pub async fn app_open(&self, path: &str) -> Result<()> {
        let params = json!({ "name": path, "path": path });
        match self.call("apps.open", params.clone()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call("computer.app_open", params).await?;
                Ok(())
            }
        }
    }

    pub async fn app_focus(&self, name: &str) -> Result<()> {
        let params = json!({ "name": name });
        match self.call("apps.focus", params.clone()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call("computer.app_focus", params).await?;
                Ok(())
            }
        }
    }

    pub async fn app_close(&self, name: &str) -> Result<()> {
        let params = json!({ "name": name });
        match self.call("apps.close", params.clone()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call("computer.app_close", params).await?;
                Ok(())
            }
        }
    }

    pub async fn app_list(&self) -> Result<Vec<Value>> {
        let res = match self.call("apps.list", json!({})).await {
            Ok(r) => r,
            Err(_) => self.call("computer.app_list", json!({})).await?,
        };
        if let Some(items) = res.get("items").and_then(|v| v.as_array()) {
            return Ok(items
                .iter()
                .map(|item| {
                    if let Some(s) = item.as_str() {
                        json!({ "name": s, "pid": Value::Null })
                    } else {
                        item.clone()
                    }
                })
                .collect());
        }
        res.get("apps")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| anyhow!("missing 'apps' in response"))
    }

    pub async fn ui_tree(
        &self,
        app_name: Option<&str>,
        max_depth: Option<usize>,
    ) -> Result<String> {
        let params = json!({
            "app": app_name,
            "app_name": app_name,
            "max_depth": max_depth,
        });
        let res = match self.call("computer.ui_tree", params.clone()).await {
            Ok(r) => r,
            Err(_) => self.call("ui.tree", params).await?,
        };
        if let Some(tree) = res.get("tree").and_then(|v| v.as_str()) {
            return Ok(tree.to_string());
        }
        if res.get("role").is_some() {
            let mut out = String::new();
            format_py_ui_element(&res, 0, &mut out);
            return Ok(out);
        }
        Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
    }

    pub async fn ui_click(&self, app_name: Option<&str>, selector: &str) -> Result<()> {
        let params = json!({
            "app_name": app_name,
            "selector": selector,
        });
        match self.call("computer.ui_click", params).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call(
                    "ui.action",
                    json!({
                        "element": {
                            "role": "",
                            "name": selector
                        },
                        "action": "click",
                        "app": app_name
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn ui_type(&self, app_name: Option<&str>, selector: &str, text: &str) -> Result<()> {
        let params = json!({
            "app_name": app_name,
            "selector": selector,
            "text": text,
        });
        match self.call("computer.ui_type", params).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.ui_click(app_name, selector).await?;
                self.keyboard_type(text).await?;
                Ok(())
            }
        }
    }

    pub async fn clipboard_read(&self) -> Result<String> {
        let res = match self.call("computer.clipboard_read", json!({})).await {
            Ok(r) => r,
            Err(_) => self.call("clipboard.read", json!({})).await?,
        };
        if let Some(text) = res.get("text").and_then(|v| v.as_str()) {
            return Ok(text.to_string());
        }
        if let Some(entries) = res.get("entries").and_then(|v| v.as_array()) {
            for entry in entries {
                if let Some(data) = entry.get("data").and_then(|v| v.as_str()) {
                    if let Ok(bytes) = data_encoding::BASE64.decode(data.as_bytes())
                        && let Ok(text) = String::from_utf8(bytes)
                    {
                        return Ok(text);
                    }
                    return Ok(data.to_string());
                }
            }
        }
        bail!("no text found in clipboard response")
    }

    pub async fn clipboard_write(&self, text: &str) -> Result<()> {
        match self
            .call("computer.clipboard_write", json!({ "text": text }))
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                self.call(
                    "clipboard.write",
                    json!({
                        "contents": {
                            "entries": [{
                                "mime_type": "text/plain;charset=utf-8",
                                "data": text
                            }]
                        }
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }
}

fn format_py_ui_element(el: &Value, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let role = el.get("role").and_then(|v| v.as_str()).unwrap_or("element");
    let name = el.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let value = el.get("value").and_then(|v| v.as_str());

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

    if let Some(children) = el.get("children").and_then(|v| v.as_array()) {
        for child in children {
            format_py_ui_element(child, depth + 1, out);
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

    let (mut transport, _server_pubkey) = if let Some(r_url) = rendezvous_url {
        let target_pk = target_pubkey
            .ok_or_else(|| anyhow!("target_pubkey is required when pairing via rendezvous"))?;
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
        let host = host.ok_or_else(|| anyhow!("host is required for direct pairing"))?;
        let port = port.unwrap_or(8423);
        let ws_url = format!("ws://{}:{}", host, port);
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .with_context(|| format!("failed to connect to {}", ws_url))?;
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
        let host = host.unwrap_or("127.0.0.1");
        let port = port.unwrap_or(8423);
        trusted.add(&verified_pubkey, &peer_name, "")?;
        trusted.cache_endpoint(&verified_pubkey, host, port)?;
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

    let peer = trusted
        .find_by_name_or_key(&target_name)
        .ok_or_else(|| anyhow!("no trusted peer found matching '{}'", target_name))?;

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
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .with_context(|| format!("failed to connect to {}", ws_url))?;
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
