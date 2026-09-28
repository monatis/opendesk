use crate::computer::local::LocalComputer;
use anyhow::{anyhow, Result};
use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::error;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

use std::collections::HashMap;
use tokio::sync::Mutex;
use crate::protocol::storage::TrustedPeers;
use crate::remote::client::{connect as remote_connect, RemoteComputer};

pub struct McpServer {
    computer: Arc<LocalComputer>,
    trusted: TrustedPeers,
    remote_peers: Arc<Mutex<HashMap<String, Arc<RemoteComputer>>>>,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    pub fn new() -> Self {
        Self {
            computer: Arc::new(LocalComputer::new()),
            trusted: TrustedPeers::new(None),
            remote_peers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn get_remote(&self, peer_name: &str) -> Result<Arc<RemoteComputer>> {
        let mut map = self.remote_peers.lock().await;
        if let Some(r) = map.get(peer_name) {
            return Ok(r.clone());
        }
        let client = remote_connect(Some(peer_name), None, None, None).await?;
        let client_arc = Arc::new(client);
        map.insert(peer_name.to_string(), client_arc.clone());
        Ok(client_arc)
    }

    pub async fn run_stdio(self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let req: JsonRpcRequest = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to parse JSON-RPC line: {e}");
                    continue;
                }
            };

            let id = match req.id {
                Some(ref i) => i.clone(),
                None => continue, // Notification, no response needed
            };

            let resp = match self.handle_method(&req.method, &req.params).await {
                Ok(res) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(res),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: None,
                    error: Some(json!({
                        "code": -32603,
                        "message": err.to_string(),
                    })),
                },
            };

            let resp_bytes = serde_json::to_vec(&resp)?;
            stdout.write_all(&resp_bytes).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }

        Ok(())
    }

    pub async fn handle_method(&self, method: &str, params: &Value) -> Result<Value> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "opendesk",
                    "version": "0.3.0"
                },
                "capabilities": {
                    "tools": {}
                }
            })),

            "notifications/initialized" => Ok(json!({})),

            "tools/list" => Ok(json!({
                "tools": self.list_tools()
            })),

            "tools/call" => {
                let tool_name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing tool name"))?;

                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let (content, is_error) = match self.call_tool(tool_name, &args).await {
                    Ok(c) => (c, false),
                    Err(e) => (
                        vec![json!({
                            "type": "text",
                            "text": format!("Error: {e}")
                        })],
                        true,
                    ),
                };

                Ok(json!({
                    "content": content,
                    "isError": is_error
                }))
            }

            _ => Err(anyhow!("Method not found: {method}")),
        }
    }

    fn list_tools(&self) -> Vec<Value> {
        vec![
            json!({
                "name": "screenshot",
                "description": "Capture a screenshot of the computer screen (full screen or a specific region).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "region": {
                            "type": "object",
                            "description": "Optional bounding rectangle {x, y, width, height}",
                            "properties": {
                                "x": { "type": "integer" },
                                "y": { "type": "integer" },
                                "width": { "type": "integer" },
                                "height": { "type": "integer" }
                            },
                            "required": ["x", "y", "width", "height"]
                        }
                    }
                }
            }),
            json!({
                "name": "mouse",
                "description": "Control the mouse: move cursor, click, double click, right click, drag, or scroll.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["move", "click", "double_click", "right_click", "drag", "scroll"],
                            "description": "The mouse action to perform."
                        },
                        "x": { "type": "integer", "description": "Target X coordinate." },
                        "y": { "type": "integer", "description": "Target Y coordinate." },
                        "button": { "type": "string", "enum": ["left", "right", "middle"], "default": "left" },
                        "to_x": { "type": "integer", "description": "Drag destination X coordinate." },
                        "to_y": { "type": "integer", "description": "Drag destination Y coordinate." },
                        "dy": { "type": "integer", "description": "Scroll delta (positive: down, negative: up)." }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "keyboard",
                "description": "Type text, press a key, or send a keyboard hotkey combination.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["type", "press", "hotkey"] },
                        "text": { "type": "string", "description": "Text to type." },
                        "key": { "type": "string", "description": "Single key name (e.g. 'Enter', 'Escape', 'Tab', 'Backspace', 'F5')." },
                        "keys": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Key sequence for hotkey chord (e.g. ['Ctrl', 's'] or ['Meta', 'Shift', 'p'])."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "app",
                "description": "Manage applications: open by name/path, focus existing window, close, or list running apps.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["open", "focus", "close", "list"] },
                        "name": { "type": "string", "description": "Application name or executable (e.g. 'notepad.exe', 'Calculator')." }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "ui",
                "description": "Direct accessibility tree interaction via Playwright-style locators without needing pixel coordinates.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["tree", "click", "type"] },
                        "app": { "type": "string", "description": "Application name to target (defaults to foreground app)." },
                        "selector": { "type": "string", "description": "Accessibility selector (e.g. 'button[name=\"Save\"]', 'input')." },
                        "text": { "type": "string", "description": "Text to type when action='type'." },
                        "max_depth": { "type": "integer", "description": "Depth limit for tree inspection (default: 8)." }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "clipboard",
                "description": "Read from or write to the system clipboard.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["read", "write"] },
                        "text": { "type": "string", "description": "Text to write to clipboard." }
                    },
                    "required": ["action"]
                }
            }),
        ]
    }

    async fn call_tool(&self, name: &str, args: &Value) -> Result<Vec<Value>> {
        let peer_arg = args.get("peer").and_then(|v| v.as_str());
        if peer_arg != Some("local") {
            let default_peer = self.trusted.get_default();
            if let Some(target) = peer_arg.or(default_peer.as_deref()) {
                return self.call_remote_tool(target, name, args).await;
            }
        }

        match name {
            "screenshot" => {
                let region = args.get("region").map(|r| xa11y::Rect {
                        x: r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        y: r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        width: r.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        height: r.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    });

                let png_bytes = self.computer.screenshot(region)?;
                let b64 = BASE64.encode(&png_bytes);

                Ok(vec![
                    json!({
                        "type": "text",
                        "text": format!("Screenshot captured ({} bytes).", png_bytes.len())
                    }),
                    json!({
                        "type": "image",
                        "data": b64,
                        "mimeType": "image/png"
                    }),
                ])
            }

            "mouse" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;

                match action {
                    "move" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        self.computer.mouse_move(x, y)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Moved mouse to ({x}, {y})") })])
                    }
                    "click" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let button = args.get("button").and_then(|v| v.as_str());
                        self.computer.mouse_click(x, y, button)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Clicked at ({x}, {y})") })])
                    }
                    "double_click" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        self.computer.mouse_double_click(x, y)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Double clicked at ({x}, {y})") })])
                    }
                    "right_click" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        self.computer.mouse_click(x, y, Some("right"))?;
                        Ok(vec![json!({ "type": "text", "text": format!("Right clicked at ({x}, {y})") })])
                    }
                    "drag" => {
                        let from_x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let from_y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let to_x = args.get("to_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let to_y = args.get("to_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        self.computer.mouse_drag(from_x, from_y, to_x, to_y)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Dragged from ({from_x}, {from_y}) to ({to_x}, {to_y})") })])
                    }
                    "scroll" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let dy = args.get("dy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        self.computer.mouse_scroll(x, y, dy)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Scrolled at ({x}, {y}) with dy={dy}") })])
                    }
                    _ => Err(anyhow!("Unknown mouse action: {action}")),
                }
            }

            "keyboard" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;

                match action {
                    "type" => {
                        let text = args
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing text"))?;
                        self.computer.keyboard_type(text)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Typed text ({} chars)", text.len()) })])
                    }
                    "press" => {
                        let key = args
                            .get("key")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing key"))?;
                        self.computer.keyboard_press(key)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Pressed key '{key}'") })])
                    }
                    "hotkey" => {
                        let keys = args
                            .get("keys")
                            .and_then(|v| v.as_array())
                            .ok_or_else(|| anyhow!("missing keys array"))?;
                        let key_strs: Vec<&str> = keys.iter().filter_map(|v| v.as_str()).collect();
                        self.computer.keyboard_hotkey(&key_strs)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Sent hotkey chord: {:?}", key_strs) })])
                    }
                    _ => Err(anyhow!("Unknown keyboard action: {action}")),
                }
            }

            "app" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;

                match action {
                    "open" => {
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing name"))?;
                        self.computer.app_open(name)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Opened application '{name}'") })])
                    }
                    "focus" => {
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing name"))?;
                        self.computer.app_focus(name)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Focused application '{name}'") })])
                    }
                    "close" => {
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing name"))?;
                        self.computer.app_close(name)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Closed application '{name}'") })])
                    }
                    "list" => {
                        let apps = self.computer.app_list()?;
                        let formatted: Vec<String> = apps
                            .into_iter()
                            .map(|a| format!("- {} (PID: {:?})", a.name, a.pid))
                            .collect();
                        Ok(vec![json!({
                            "type": "text",
                            "text": format!("Running accessible applications:\n{}", formatted.join("\n"))
                        })])
                    }
                    _ => Err(anyhow!("Unknown app action: {action}")),
                }
            }

            "ui" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;
                let app = args.get("app").and_then(|v| v.as_str());

                match action {
                    "tree" => {
                        let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).map(|d| d as usize);
                        let dump = self.computer.ui_tree(app, max_depth)?;
                        Ok(vec![json!({ "type": "text", "text": dump })])
                    }
                    "click" => {
                        let selector = args
                            .get("selector")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing selector"))?;
                        self.computer.ui_click(app, selector)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Clicked element matching '{selector}'") })])
                    }
                    "type" => {
                        let selector = args
                            .get("selector")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing selector"))?;
                        let text = args
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing text"))?;
                        self.computer.ui_type(app, selector, text)?;
                        Ok(vec![json!({ "type": "text", "text": format!("Typed text into element matching '{selector}'") })])
                    }
                    _ => Err(anyhow!("Unknown ui action: {action}")),
                }
            }

            "clipboard" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;

                match action {
                    "read" => {
                        let text = self.computer.clipboard_read()?;
                        Ok(vec![json!({ "type": "text", "text": text })])
                    }
                    "write" => {
                        let text = args
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing text"))?;
                        self.computer.clipboard_write(text)?;
                        Ok(vec![json!({ "type": "text", "text": "Text written to clipboard successfully." })])
                    }
                    _ => Err(anyhow!("Unknown clipboard action: {action}")),
                }
            }

            _ => Err(anyhow!("Unknown tool: {name}")),
        }
    }

    async fn call_remote_tool(&self, target: &str, name: &str, args: &Value) -> Result<Vec<Value>> {
        let remote = self.get_remote(target).await?;
        match name {
            "screenshot" => {
                let b64 = remote.screenshot("png", None).await?;
                Ok(vec![
                    json!({
                        "type": "text",
                        "text": format!("Screenshot captured on remote peer '{}'.", target)
                    }),
                    json!({
                        "type": "image",
                        "data": b64,
                        "mimeType": "image/png"
                    }),
                ])
            }
            "mouse" => {
                let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing action"))?;
                match action {
                    "move" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        remote.mouse_move(x, y).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Moved mouse on {target} to ({x}, {y})") })])
                    }
                    "click" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let button = args.get("button").and_then(|v| v.as_str());
                        remote.mouse_click(x, y, button).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Clicked on {target} at ({x}, {y})") })])
                    }
                    "double_click" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        remote.mouse_double_click(x, y).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Double clicked on {target} at ({x}, {y})") })])
                    }
                    "right_click" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        remote.mouse_click(x, y, Some("right")).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Right clicked on {target} at ({x}, {y})") })])
                    }
                    "drag" => {
                        let from_x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let from_y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let to_x = args.get("to_x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let to_y = args.get("to_y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        remote.mouse_drag(from_x, from_y, to_x, to_y).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Dragged on {target} from ({from_x}, {from_y}) to ({to_x}, {to_y})") })])
                    }
                    "scroll" => {
                        let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let dy = args.get("dy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        remote.mouse_scroll(x, y, dy).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Scrolled on {target} at ({x}, {y}) with dy={dy}") })])
                    }
                    _ => Err(anyhow!("Unknown mouse action: {action}")),
                }
            }
            "keyboard" => {
                let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing action"))?;
                match action {
                    "type" => {
                        let text = args.get("text").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing text"))?;
                        remote.keyboard_type(text).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Typed text on {target} ({} chars)", text.len()) })])
                    }
                    "press" => {
                        let key = args.get("key").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing key"))?;
                        remote.keyboard_press(key).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Pressed key '{key}' on {target}") })])
                    }
                    "hotkey" => {
                        let keys = args.get("keys").and_then(|v| v.as_array()).ok_or_else(|| anyhow!("missing keys array"))?;
                        let key_strs: Vec<&str> = keys.iter().filter_map(|v| v.as_str()).collect();
                        remote.keyboard_hotkey(&key_strs).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Sent hotkey chord on {target}: {:?}", key_strs) })])
                    }
                    _ => Err(anyhow!("Unknown keyboard action: {action}")),
                }
            }
            "app" => {
                let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing action"))?;
                match action {
                    "open" => {
                        let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing name"))?;
                        remote.app_open(name).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Opened application '{name}' on {target}") })])
                    }
                    "focus" => {
                        let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing name"))?;
                        remote.app_focus(name).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Focused application '{name}' on {target}") })])
                    }
                    "close" => {
                        let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing name"))?;
                        remote.app_close(name).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Closed application '{name}' on {target}") })])
                    }
                    "list" => {
                        let apps = remote.app_list().await?;
                        let formatted: Vec<String> = apps.iter().map(|a| {
                            let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let pid = a.get("pid").and_then(|v| v.as_i64()).unwrap_or(0);
                            format!("- {} (PID: {})", name, pid)
                        }).collect();
                        Ok(vec![json!({ "type": "text", "text": format!("Running accessible applications on {target}:\n{}", formatted.join("\n")) })])
                    }
                    _ => Err(anyhow!("Unknown app action: {action}")),
                }
            }
            "ui" => {
                let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing action"))?;
                let app = args.get("app").and_then(|v| v.as_str());
                match action {
                    "tree" => {
                        let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).map(|d| d as usize);
                        let dump = remote.ui_tree(app, max_depth).await?;
                        Ok(vec![json!({ "type": "text", "text": dump })])
                    }
                    "click" => {
                        let selector = args.get("selector").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing selector"))?;
                        remote.ui_click(app, selector).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Clicked element matching '{selector}' on {target}") })])
                    }
                    "type" => {
                        let selector = args.get("selector").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing selector"))?;
                        let text = args.get("text").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing text"))?;
                        remote.ui_type(app, selector, text).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Typed '{text}' into '{selector}' on {target}") })])
                    }
                    _ => Err(anyhow!("Unknown ui action: {action}")),
                }
            }
            "clipboard" => {
                let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing action"))?;
                match action {
                    "read" => {
                        let text = remote.clipboard_read().await?;
                        Ok(vec![json!({ "type": "text", "text": text })])
                    }
                    "write" => {
                        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        remote.clipboard_write(text).await?;
                        Ok(vec![json!({ "type": "text", "text": format!("Clipboard updated on {target}.") })])
                    }
                    _ => Err(anyhow!("Unknown clipboard action: {action}")),
                }
            }
            _ => Err(anyhow!("Unknown tool: {name}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_initialize() {
        let server = McpServer::new();
        let res = server.handle_method("initialize", &json!({})).await.unwrap();
        assert_eq!(res["serverInfo"]["name"], "opendesk");
    }

    #[tokio::test]
    async fn test_mcp_tools_list() {
        let server = McpServer::new();
        let res = server.handle_method("tools/list", &json!({})).await.unwrap();
        let tools = res["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "screenshot"));
        assert!(tools.iter().any(|t| t["name"] == "mouse"));
        assert!(tools.iter().any(|t| t["name"] == "keyboard"));
        assert!(tools.iter().any(|t| t["name"] == "app"));
        assert!(tools.iter().any(|t| t["name"] == "ui"));
        assert!(tools.iter().any(|t| t["name"] == "clipboard"));
    }
}
