//! RemoteComputer and client connection helpers: pair_with and connect.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;

use crate::protocol::crypto::EncryptedChannel;
use crate::protocol::frames::{Frame, HelloFrame, ReqFrame};
use crate::protocol::handshake::{auth_client, pair_client, Transport};
use crate::protocol::identity::Identity;
use crate::protocol::storage::{default_peer_name, TrustedPeers};
use super::transport::WebSocketTransport;

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
                            let err_msg = res.error.map(|e| e.to_string())
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
        let res = self.call("computer.screenshot", params).await?;
        res.get("image_base64")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("missing 'image_base64' in response"))
    }

    pub async fn mouse_move(&self, x: i32, y: i32) -> Result<()> {
        self.call("computer.mouse_move", json!({ "x": x, "y": y })).await?;
        Ok(())
    }

    pub async fn mouse_click(&self, x: i32, y: i32, button: Option<&str>) -> Result<()> {
        self.call("computer.mouse_click", json!({ "x": x, "y": y, "button": button })).await?;
        Ok(())
    }

    pub async fn mouse_double_click(&self, x: i32, y: i32) -> Result<()> {
        self.call("computer.mouse_double_click", json!({ "x": x, "y": y })).await?;
        Ok(())
    }

    pub async fn mouse_drag(&self, start_x: i32, start_y: i32, end_x: i32, end_y: i32) -> Result<()> {
        self.call("computer.mouse_drag", json!({
            "start_x": start_x,
            "start_y": start_y,
            "end_x": end_x,
            "end_y": end_y,
        })).await?;
        Ok(())
    }

    pub async fn mouse_scroll(&self, x: i32, y: i32, delta_y: i32) -> Result<()> {
        self.call("computer.mouse_scroll", json!({
            "x": x,
            "y": y,
            "delta_y": delta_y,
        })).await?;
        Ok(())
    }

    pub async fn keyboard_type(&self, text: &str) -> Result<()> {
        self.call("computer.keyboard_type", json!({ "text": text })).await?;
        Ok(())
    }

    pub async fn keyboard_press(&self, key: &str) -> Result<()> {
        self.call("computer.keyboard_press", json!({ "key": key })).await?;
        Ok(())
    }

    pub async fn keyboard_hotkey(&self, keys: &[&str]) -> Result<()> {
        self.call("computer.keyboard_hotkey", json!({ "keys": keys })).await?;
        Ok(())
    }

    pub async fn app_open(&self, path: &str) -> Result<()> {
        self.call("computer.app_open", json!({ "path": path })).await?;
        Ok(())
    }

    pub async fn app_focus(&self, name: &str) -> Result<()> {
        self.call("computer.app_focus", json!({ "name": name })).await?;
        Ok(())
    }

    pub async fn app_close(&self, name: &str) -> Result<()> {
        self.call("computer.app_close", json!({ "name": name })).await?;
        Ok(())
    }

    pub async fn app_list(&self) -> Result<Vec<Value>> {
        let res = self.call("computer.app_list", json!({})).await?;
        res.get("apps")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| anyhow!("missing 'apps' in response"))
    }

    pub async fn ui_tree(&self, app_name: Option<&str>, max_depth: Option<usize>) -> Result<String> {
        let res = self.call("computer.ui_tree", json!({
            "app_name": app_name,
            "max_depth": max_depth,
        })).await?;
        res.get("tree")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("missing 'tree' in response"))
    }

    pub async fn ui_click(&self, app_name: Option<&str>, selector: &str) -> Result<()> {
        self.call("computer.ui_click", json!({
            "app_name": app_name,
            "selector": selector,
        })).await?;
        Ok(())
    }

    pub async fn ui_type(&self, app_name: Option<&str>, selector: &str, text: &str) -> Result<()> {
        self.call("computer.ui_type", json!({
            "app_name": app_name,
            "selector": selector,
            "text": text,
        })).await?;
        Ok(())
    }

    pub async fn clipboard_read(&self) -> Result<String> {
        let res = self.call("computer.clipboard_read", json!({})).await?;
        res.get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("missing 'text' in response"))
    }

    pub async fn clipboard_write(&self, text: &str) -> Result<()> {
        self.call("computer.clipboard_write", json!({ "text": text })).await?;
        Ok(())
    }
}

/// Pair with a remote peer running `opendesk pair`.
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
        let pk_bytes = data_encoding::HEXLOWER.decode(target_pk.as_bytes())
            .or_else(|_| data_encoding::HEXUPPER.decode(target_pk.as_bytes()))
            .with_context(|| format!("invalid target_pubkey hex: {}", target_pk))?;
        if pk_bytes.len() != 32 {
            return Err(anyhow!("target_pubkey must be 32 bytes"));
        }
        let mut target_arr = [0u8; 32];
        target_arr.copy_from_slice(&pk_bytes);

        let tr = super::rendezvous::connect_via_rendezvous(r_url, rendezvous_token, &target_arr).await?;
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
                && let Some(desc) = desc_val.as_str() {
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
        let mut tr = super::rendezvous::connect_via_rendezvous(r_url, rendezvous_token, &peer_pub).await?;
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
                && let Some(desc) = desc_val.as_str() {
                    let _ = trusted.cache_description(&peer_pub, desc);
                }
            h.capabilities
        }
        _ => HashMap::new(),
    };

    Ok(RemoteComputer::new(transport, channel, caps))
}
