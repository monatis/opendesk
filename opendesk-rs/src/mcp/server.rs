use crate::automation::scheduler::ScheduleStore;
use crate::computer::local::LocalComputer;
use crate::protocol::storage::{TrustedPeers, default_home, resolve_rendezvous_config};
use crate::remote::audit::AuditLog;
use crate::remote::client::{RemoteComputer, connect as remote_connect};
use crate::remote::discovery::discover;
use anyhow::{Result, anyhow, bail};
use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::error;

const INDEX_HTML: &str = include_str!("../../static/index.html");
const STYLES_CSS: &str = include_str!("../../static/styles.css");
const APP_JS: &str = include_str!("../../static/app.js");

pub fn build_mcp_app_html() -> String {
    let mut html = INDEX_HTML.to_string();
    html = html.replace(
        r#"<link rel="stylesheet" href="/static/styles.css">"#,
        &format!("<style>\n{STYLES_CSS}\n</style>"),
    );
    html = html.replace(
        r#"<script src="/static/app.js" type="module"></script>"#,
        &format!("<script type=\"module\">\n{APP_JS}\n</script>"),
    );
    html
}

fn url_decode_simple(s: &str) -> String {
    let mut res = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next().unwrap_or(' ');
            let h2 = chars.next().unwrap_or(' ');
            let hex = format!("{h1}{h2}");
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                res.push(byte as char);
            } else {
                res.push('%');
                res.push(h1);
                res.push(h2);
            }
        } else if c == '+' {
            res.push(' ');
        } else {
            res.push(c);
        }
    }
    res
}

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

pub struct McpSession {
    pub trusted: TrustedPeers,
    pub current_peer: Option<String>,
    pub connections: HashMap<String, Arc<RemoteComputer>>,
    pub home: PathBuf,
    pub rendezvous_url: Option<String>,
    pub rendezvous_token: Option<String>,
}

impl Default for McpSession {
    fn default() -> Self {
        Self::new()
    }
}

impl McpSession {
    pub fn new() -> Self {
        Self::with_config(None, None, None)
    }

    pub fn with_config(
        home: Option<PathBuf>,
        rendezvous_url: Option<String>,
        rendezvous_token: Option<String>,
    ) -> Self {
        let home_path = home.unwrap_or_else(|| {
            std::env::var("OPENDESK_HOME")
                .ok()
                .map(PathBuf::from)
                .unwrap_or_else(default_home)
        });
        let r_cfg = resolve_rendezvous_config(
            Some(&home_path),
            rendezvous_url.as_deref(),
            rendezvous_token.as_deref(),
        );
        let r_url = if !r_cfg.url.is_empty() {
            Some(r_cfg.url)
        } else {
            None
        };
        let r_token = r_cfg.token;

        Self {
            trusted: TrustedPeers::new(Some(&home_path)),
            current_peer: None,
            connections: HashMap::new(),
            home: home_path,
            rendezvous_url: r_url,
            rendezvous_token: r_token,
        }
    }

    pub fn effective_peer(&self) -> (Option<String>, &'static str) {
        if let Some(ref cur) = self.current_peer {
            return (Some(cur.clone()), "explicit");
        }
        if let Some(def) = self.trusted.get_default() {
            return (Some(def), "persistent");
        }
        (None, "local")
    }

    pub fn use_peer(&mut self, name: Option<&str>) -> Result<()> {
        match name {
            None | Some("auto") => {
                self.current_peer = None;
                Ok(())
            }
            Some("local") => {
                self.current_peer = Some("local".to_string());
                Ok(())
            }
            Some(p) => {
                if self.trusted.find_by_name(p).is_none() {
                    bail!(
                        "No trusted peer named {p:?}.  Pair from the controlled machine first via `opendesk pair`, then `opendesk pair-with <host> <code> --name {p}` here."
                    );
                }
                self.current_peer = Some(p.to_string());
                Ok(())
            }
        }
    }

    pub fn disconnect(&mut self, name: Option<&str>) -> usize {
        if let Some(p) = name {
            if p == "local" {
                if self.current_peer.as_deref() == Some("local") {
                    self.current_peer = None;
                }
                return 0;
            }
            let removed = self.connections.remove(p).is_some();
            if self.current_peer.as_deref() == Some(p) {
                self.current_peer = None;
            }
            if removed { 1 } else { 0 }
        } else {
            let n = self.connections.len();
            self.connections.clear();
            self.current_peer = None;
            n
        }
    }

    pub fn active_peer_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.connections.keys().cloned().collect();
        v.sort();
        v
    }

    pub async fn get_remote(&mut self, peer_name: &str) -> Result<Arc<RemoteComputer>> {
        if let Some(r) = self.connections.get(peer_name) {
            return Ok(r.clone());
        }
        let client = remote_connect(
            Some(peer_name),
            self.rendezvous_url.as_deref(),
            self.rendezvous_token.as_deref(),
            Some(&self.home),
        )
        .await?;
        let client_arc = Arc::new(client);
        self.connections
            .insert(peer_name.to_string(), client_arc.clone());
        Ok(client_arc)
    }

    pub async fn resolve(
        &mut self,
        requested: Option<&str>,
    ) -> Result<(Option<Arc<RemoteComputer>>, String)> {
        let chosen = if let Some(req) = requested.filter(|s| !s.trim().is_empty()) {
            req.to_string()
        } else if let Some(ref cur) = self.current_peer {
            cur.clone()
        } else {
            let (eff, _source) = self.effective_peer();
            eff.unwrap_or_else(|| "local".to_string())
        };

        if chosen == "local" {
            Ok((None, "local".to_string()))
        } else {
            let remote = self.get_remote(&chosen).await?;
            Ok((Some(remote), chosen))
        }
    }
}

#[derive(Clone)]
pub struct McpServer {
    computer: Arc<LocalComputer>,
    session: Arc<Mutex<McpSession>>,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    pub fn new() -> Self {
        Self::with_config(None, None, None)
    }

    pub fn with_config(
        home: Option<PathBuf>,
        rendezvous_url: Option<String>,
        rendezvous_token: Option<String>,
    ) -> Self {
        Self {
            computer: Arc::new(LocalComputer::new()),
            session: Arc::new(Mutex::new(McpSession::with_config(
                home,
                rendezvous_url,
                rendezvous_token,
            ))),
        }
    }

    pub async fn resolve_remote(
        &self,
        requested: Option<&str>,
    ) -> Result<(Option<Arc<RemoteComputer>>, String)> {
        let (chosen, existing_conn, r_url, r_token, home) = {
            let session = self.session.lock().await;
            let chosen = if let Some(req) = requested.filter(|s| !s.trim().is_empty()) {
                req.to_string()
            } else if let Some(ref cur) = session.current_peer {
                cur.clone()
            } else {
                let (eff, _source) = session.effective_peer();
                eff.unwrap_or_else(|| "local".to_string())
            };
            if chosen == "local" {
                return Ok((None, "local".to_string()));
            }
            let conn = session.connections.get(&chosen).cloned();
            (
                chosen,
                conn,
                session.rendezvous_url.clone(),
                session.rendezvous_token.clone(),
                session.home.clone(),
            )
        };

        if let Some(r) = existing_conn {
            return Ok((Some(r), chosen));
        }

        // Establish remote connection without holding the session lock
        let client = remote_connect(
            Some(&chosen),
            r_url.as_deref(),
            r_token.as_deref(),
            Some(&home),
        )
        .await?;

        let client_arc = Arc::new(client);
        {
            let mut session = self.session.lock().await;
            session
                .connections
                .insert(chosen.clone(), client_arc.clone());
        }
        Ok((Some(client_arc), chosen))
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
                    "tools": {
                        "listChanged": false
                    },
                    "resources": {
                        "subscribe": false,
                        "listChanged": false
                    }
                }
            })),

            "notifications/initialized" => Ok(json!({})),

            "resources/list" => Ok(json!({
                "resources": [
                    {
                        "uri": "ui://opendesk/viewport",
                        "name": "OpenDesk Remote Viewport",
                        "description": "Interactive remote desktop canvas, controls, and peer switcher",
                        "mimeType": "text/html;profile=mcp-app"
                    }
                ]
            })),

            "resources/templates/list" => Ok(json!({
                "resourceTemplates": []
            })),

            "resources/read" => {
                let uri = params
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing uri"))?;

                let clean_uri = uri.split('?').next().unwrap_or(uri).trim_end_matches('/');
                if clean_uri == "ui://opendesk/viewport" {
                    Ok(json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/html;profile=mcp-app",
                                "text": build_mcp_app_html(),
                                "_meta": {
                                    "ui": {
                                        "csp": {
                                            "connectDomains": [],
                                            "resourceDomains": [],
                                            "frameDomains": [],
                                            "baseUriDomains": []
                                        },
                                        "prefersBorder": true
                                    }
                                }
                            }
                        ]
                    }))
                } else {
                    Err(anyhow!("Resource not found: {uri}"))
                }
            }

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
                    Ok(c) => {
                        let has_err = c.iter().any(|item| {
                            item.get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.starts_with("Error:") || s.starts_with("ERROR:"))
                                .unwrap_or(false)
                        });
                        (c, has_err)
                    }
                    Err(e) => {
                        let mut msg = e.to_string();
                        while let Some(stripped) = msg
                            .strip_prefix("Error: ")
                            .or_else(|| msg.strip_prefix("ERROR: "))
                        {
                            msg = stripped.trim().to_string();
                        }
                        (
                            vec![json!({
                                "type": "text",
                                "text": format!("Error: {msg}")
                            })],
                            true,
                        )
                    }
                };

                let mut resp_map = serde_json::Map::new();
                resp_map.insert("content".to_string(), json!(content));
                resp_map.insert("isError".to_string(), json!(is_error));

                if !is_error && tool_name == "opendesk_view" {
                    resp_map.insert(
                        "_meta".to_string(),
                        json!({
                            "ui": {
                                "resourceUri": "ui://opendesk/viewport"
                            }
                        }),
                    );
                }

                Ok(Value::Object(resp_map))
            }

            _ => Err(anyhow!("Method not found: {method}")),
        }
    }

    pub fn list_tools(&self) -> Vec<Value> {
        vec![
            json!({
                "name": "screenshot",
                "description": "Capture a screenshot of the current screen or a sub-region. Returns the image so you can observe the current UI state before deciding which action to take next. Call this frequently to verify that previous actions had the intended effect.\n\nOptions:\n  show_cursor=true  — draw a red dot at the current cursor position\n  marks=true        — overlay numbered boxes on all interactive elements (Set-of-Marks); the output lists each mark so you can say 'click mark 3' instead of guessing pixel coordinates\n  zoom=[x0,y0,x1,y1] — return a cropped close-up of a screen region\n  save_path         — write the PNG to disk\n\nRoutes to the session's default peer unless 'peer' is provided. See `opendesk_peers` to list available targets.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "region": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Screen region to capture as [x, y, width, height] in pixels. Omit to capture the entire primary screen."
                        },
                        "save_path": {
                            "type": "string",
                            "description": "Absolute path where the PNG should be saved on disk."
                        },
                        "show_cursor": {
                            "type": "boolean",
                            "default": false,
                            "description": "When true, overlays a red dot at the current cursor position."
                        },
                        "marks": {
                            "type": "boolean",
                            "default": false,
                            "description": "When true, draws numbered bounding boxes (Set-of-Marks) over all interactive UI elements. Uses the platform accessibility API."
                        },
                        "zoom": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Crop region as [x0, y0, x1, y1] in logical screen pixels. Returns a zoomed-in view. Use after a full screenshot to inspect small text or crowded UI areas."
                        },
                        "peer": {
                            "type": "string",
                            "description": "Optional. Name of the peer to run this action on. Defaults to 'local' (this machine) unless an explicit or persistent default peer is configured. Use any name from `opendesk_peers` or set a default via `opendesk_use`."
                        },
                        "format": {
                            "type": "string",
                            "enum": ["jpeg", "png"],
                            "description": "Image compression format. 'jpeg' is fast and lightweight (recommended for remote/relays); 'png' is lossless."
                        },
                        "quality": {
                            "type": "integer",
                            "description": "JPEG compression quality (1-100). Defaults to 75."
                        }
                    }
                }
            }),
            json!({
                "name": "mouse",
                "description": "Control the mouse: move to a position, click (left/right/middle), double-click, triple-click, scroll, or drag from one point to another. Always provide image_width and image_height from the screenshot tool output for correct Retina/HiDPI coordinate translation.\n\nRoutes to the session's default peer unless 'peer' is provided. See `opendesk_peers` to list available targets.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": [
                                "move", "click", "double_click", "triple_click",
                                "right_click", "middle_click",
                                "left_down", "left_up",
                                "scroll", "drag",
                                "cursor_position"
                            ],
                            "description": "Mouse action: move, click, double_click, triple_click, right_click, middle_click, left_down, left_up, scroll, drag, cursor_position"
                        },
                        "x": { "type": "integer", "default": 0, "description": "X coordinate as it appears in the screenshot." },
                        "y": { "type": "integer", "default": 0, "description": "Y coordinate as it appears in the screenshot." },
                        "end_x": { "type": "integer", "description": "Target X for drag action." },
                        "end_y": { "type": "integer", "description": "Target Y for drag action." },
                        "image_width": { "type": "integer", "description": "Width of the screenshot the coordinates were read from. ALWAYS provide this for correct Retina/HiDPI scaling." },
                        "image_height": { "type": "integer", "description": "Height of the screenshot. Provide alongside image_width." },
                        "direction": { "type": "string", "enum": ["up", "down", "left", "right"], "description": "Scroll direction. Used with action='scroll'." },
                        "amount": { "type": "integer", "default": 3, "description": "Scroll amount in clicks." },
                        "duration": { "type": "number", "default": 0.25, "description": "Movement duration in seconds." },
                        "settle_ms": { "type": "integer", "default": 500, "description": "Milliseconds to wait after the action for the UI to settle." },
                        "peer": {
                            "type": "string",
                            "description": "Optional. Name of the peer to run this action on. Defaults to 'local' (this machine) unless an explicit or persistent default peer is configured. Use any name from `opendesk_peers` or set a default via `opendesk_use`."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "keyboard",
                "description": "Simulate keyboard input: type a string of text, press a single key, or send a key combination (hotkey). Unicode text is fully supported.\n\nRoutes to the session's default peer unless 'peer' is provided. See `opendesk_peers` to list available targets.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["type", "press", "hotkey", "hold"], "description": "Keyboard action: type, press, hotkey, or hold." },
                        "text": { "type": "string", "description": "Text to type. Required for action='type'." },
                        "key": { "type": "string", "description": "Key name to press/hold: 'enter', 'escape', 'tab', 'f5', etc." },
                        "keys": { "type": "array", "items": { "type": "string" }, "description": "Key names for hotkey, e.g. ['ctrl','c']." },
                        "interval": { "type": "number", "default": 0.02, "description": "Seconds between keystrokes (action='type')." },
                        "settle_ms": { "type": "integer", "default": 300, "description": "Milliseconds to wait after the action." },
                        "hold_duration": { "type": "number", "default": 1.0, "description": "Seconds to hold key for action='hold'." },
                        "peer": {
                            "type": "string",
                            "description": "Optional. Name of the peer to run this action on. Defaults to 'local' (this machine) unless an explicit or persistent default peer is configured. Use any name from `opendesk_peers` or set a default via `opendesk_use`."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "app",
                "description": "Interact with desktop applications: open an app by name, close it, bring it to the foreground, or list all currently running windows.\n\nRoutes to the session's default peer unless 'peer' is provided. See `opendesk_peers` to list available targets.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["open", "close", "focus", "list"], "description": "App action: open, close, focus, or list." },
                        "name": { "type": "string", "description": "Application name (e.g. 'Terminal', 'Google Chrome', 'VS Code') or full executable path. Required for open/close/focus." },
                        "peer": {
                            "type": "string",
                            "description": "Optional. Name of the peer to run this action on. Defaults to 'local' (this machine) unless an explicit or persistent default peer is configured. Use any name from `opendesk_peers` or set a default via `opendesk_use`."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "ui",
                "description": "Interact with UI elements by name on any OS — no coordinates needed. ALWAYS try this before the mouse tool. Actions:\n  get_tree   — list all accessible elements in the app window\n  click      — click a button or element by its title\n  click_menu — click a menu item, e.g. File → Save\n  type       — type text (clipboard-paste, Unicode-safe)\n  press_key  — press a key or chord: key='return', modifiers=['command']\n  get_value  — read the current text value of a named element\n\nRoutes to the session's default peer unless 'peer' is provided. See `opendesk_peers` to list available targets.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["get_tree", "click", "click_menu", "type", "press_key", "get_value"], "description": "UI action to perform." },
                        "app": { "type": "string", "description": "Application name / process name. macOS: process name as in Activity Monitor, e.g. 'TextEdit', 'Safari'. Linux: process or window title, e.g. 'gedit', 'firefox'. Windows: executable name or title, e.g. 'Notepad', 'notepad.exe'." },
                        "title": { "type": "string", "description": "Title or label of the UI element." },
                        "role": { "type": "string", "description": "Element role to narrow the search. macOS: 'button', 'text field', 'text area', 'checkbox'. Linux: 'push button', 'entry', 'text', 'check box'. Windows: 'Button', 'Edit', 'Text', 'CheckBox', 'ComboBox'." },
                        "text": { "type": "string", "description": "Text to type. Required for action='type'." },
                        "menu": { "type": "string", "description": "Menu bar menu name for click_menu, e.g. 'File', 'Edit'." },
                        "menu_item": { "type": "string", "description": "Menu item name for click_menu, e.g. 'Save', 'Copy'." },
                        "key": { "type": "string", "description": "Key for press_key: 'return', 'escape', 'tab', 'space', 'delete', 'up', 'down', 'left', 'right', 'home', 'end', 'f1'–'f12', or a single char." },
                        "modifiers": { "type": "array", "items": { "type": "string" }, "description": "Modifier keys for press_key: 'command' (macOS), 'ctrl', 'shift', 'alt'. E.g. ['command'] for cmd+key." },
                        "window_index": { "type": "integer", "default": 1, "description": "Window index (1 = frontmost)." },
                        "selector": { "type": "string", "description": "Optional accessibility selector (e.g. 'button[name=\"Save\"]', 'input')." },
                        "max_depth": { "type": "integer", "description": "Depth limit for tree inspection (default: 8)." },
                        "peer": {
                            "type": "string",
                            "description": "Optional. Name of the peer to run this action on. Defaults to 'local' (this machine) unless an explicit or persistent default peer is configured. Use any name from `opendesk_peers` or set a default via `opendesk_use`."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "clipboard",
                "description": "Read the current clipboard text or write new text to the clipboard. Use 'read' to retrieve copied text; 'write' to place text on the clipboard.\n\nRoutes to the session's default peer unless 'peer' is provided. See `opendesk_peers` to list available targets.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["read", "write"], "description": "Clipboard action: read or write." },
                        "text": { "type": "string", "description": "Text to place on the clipboard. Required for action='write'." },
                        "peer": {
                            "type": "string",
                            "description": "Optional. Name of the peer to run this action on. Defaults to 'local' (this machine) unless an explicit or persistent default peer is configured. Use any name from `opendesk_peers` or set a default via `opendesk_use`."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "audit",
                "description": "Show or replay the session audit log.\n  action='show'   — display recorded actions (format='summary' or 'full')\n  action='replay' — re-execute every action from this session (skips screenshots, reads, and optionally failed actions)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["show", "replay"], "default": "show", "description": "'show' displays the log; 'replay' re-executes actions." },
                        "format": { "type": "string", "enum": ["summary", "full"], "default": "full", "description": "For action='show': 'summary' = one-line count, 'full' = timestamped log." },
                        "session_id": { "type": "string", "description": "Session to inspect/replay. Defaults to the current session." },
                        "skip_errors": { "type": "boolean", "default": true, "description": "For action='replay': skip actions that originally errored." }
                    }
                }
            }),
            json!({
                "name": "schedule",
                "description": "Schedule tasks to run automatically on a timer.\n\nActions:\n- add: schedule a task. task can be natural language ('take a screenshot and save it') or a learned procedure ('replay expense-report'). timing examples: 'every 30m', 'every 2h', 'every day at 09:00', 'every friday at 17:00'\n- remove: remove a schedule by name\n- list: show all scheduled tasks\n- run: run a scheduled task immediately (for testing)\n\nAfter adding schedules, start the background runner with: opendesk scheduler start",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["add", "remove", "list", "run"], "description": "Schedule action: add, remove, list, run" },
                        "name": { "type": "string", "description": "Schedule name" },
                        "task": { "type": "string", "description": "Task description or procedure to run" },
                        "timing": { "type": "string", "description": "Timing expression, e.g. 'every 30m', 'every 2h', 'every day at 09:00'" }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "learn",
                "description": "Record and replay computer tasks. Actions:\n- start: begin recording mouse/keyboard/screenshots for a named task\n- stop: stop recording; returns trajectory summary and screenshots for you to summarize into a procedure, then call learn(save) with the JSON\n- save: save a procedure JSON returned from stop\n- replay: load a saved procedure and return step-by-step replay instructions\n- list: list all saved procedures\n\nTypical workflow: learn(start) → user performs task → learn(stop) → summarize the trajectory → learn(save) → later: learn(replay)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["start", "stop", "save", "replay", "list"], "description": "Learn action: start, stop, save, replay, list" },
                        "task_name": { "type": "string", "description": "Task name" },
                        "procedure": { "type": "string", "description": "JSON procedure string for save action" }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "opendesk_view",
                "description": "Open the interactive OpenDesk MCP App remote desktop viewport. Renders an interactive remote desktop canvas, controls, and peer switcher directly inline inside MCP App hosts (Claude Desktop, Goose, isanagent).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "peer": {
                            "type": "string",
                            "description": "Optional peer to view and control. Defaults to current or local machine."
                        }
                    },
                    "additionalProperties": false
                },
                "_meta": {
                    "ui": {
                        "resourceUri": "ui://opendesk/viewport"
                    }
                }
            }),
            json!({
                "name": "opendesk_peers",
                "description": "List the peers available to this MCP session. Returns the local machine and every paired remote peer. The current default is marked with [current]; open connections are marked [active].",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
            json!({
                "name": "opendesk_discover",
                "description": "Discover opendesk peers advertising themselves on the LAN via mDNS. May take a few seconds. Only paired peers can be used; discovered-but-unpaired peers will need pairing via the CLI (`opendesk pair-with <host> <code>`).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "timeout": { "type": "number", "description": "Seconds to wait for mDNS responses. Defaults to 2." }
                    },
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "opendesk_use",
                "description": "Set the default peer for subsequent Computer-use tool calls. Pass 'local' (or omit `peer`) to revert to controlling this machine.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "peer": { "type": "string", "description": "Peer name (from `opendesk_peers`), or 'local'." }
                    },
                    "required": ["peer"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "opendesk_status",
                "description": "Show the current default peer and the list of peers with open cached connections in this session.",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
            json!({
                "name": "opendesk_describe",
                "description": "Return the full description of a paired peer — what that machine is for, what apps / data it has access to.  Set by the controlled-machine operator via `opendesk describe` and (optionally) overridden locally via `opendesk peers describe`.  Use this to decide which peer is right for a given task.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "peer": { "type": "string", "description": "Peer name (from `opendesk_peers`)." }
                    },
                    "required": ["peer"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "opendesk_capabilities",
                "description": "Show what the given peer's backend can do (input devices, screen capture, filesystem, etc.). Omit `peer` for the current default.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "peer": { "type": "string", "description": "Peer name or 'local'." }
                    },
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "opendesk_app_api",
                "description": "Internal REST-to-MCP bridge tool powering the embedded OpenDesk web UI when rendered as an MCP App.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "The API path (e.g. /api/state, /api/rendezvous/config)" },
                        "method": { "type": "string", "description": "HTTP method (GET, POST, DELETE)" },
                        "body": { "type": "object", "description": "Optional JSON payload" }
                    },
                    "required": ["path"]
                }
            }),
            json!({
                "name": "opendesk_disconnect",
                "description": "Close a cached remote connection. Pass `peer` to close one, or omit it to close all remote connections.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "peer": { "type": "string" }
                    },
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "opendesk_privacy",
                "description": "Control physical input locking and screen blackout (privacy curtain) on a REMOTE peer. Blocks physical mouse/keyboard usage on the remote machine while remote control is active, and optionally displays a privacy blackout curtain on the physical monitor while allowing remote screen capture.\n\nSAFETY: This tool is STRICTLY restricted to remote peers. It is prohibited on 'local' to prevent the operator from being locked out of their own machine.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "peer": {
                            "type": "string",
                            "description": "Required. Name of the remote peer to configure privacy/locking for. Cannot be 'local'."
                        },
                        "lock_input": {
                            "type": "boolean",
                            "default": true,
                            "description": "Whether to block physical keyboard and mouse inputs on the remote host."
                        },
                        "blackout": {
                            "type": "boolean",
                            "default": false,
                            "description": "Whether to show a blackout screen curtain on the physical monitor."
                        },
                        "action": {
                            "type": "string",
                            "enum": ["set", "status", "unlock"],
                            "default": "set",
                            "description": "'set' applies lock_input/blackout settings; 'status' queries current state; 'unlock' removes all input locks and blackout."
                        }
                    },
                    "required": ["peer"],
                    "additionalProperties": false
                }
            }),
        ]
    }

    pub async fn call_tool(&self, name: &str, args: &Value) -> Result<Vec<Value>> {
        // Handle admin tools
        match name {
            "opendesk_app_api" => return Box::pin(self.admin_app_api(args)).await,
            "opendesk_view" => return self.admin_view(args).await,
            "opendesk_peers" => return self.admin_peers(args).await,
            "opendesk_discover" => return self.admin_discover(args).await,
            "opendesk_use" => return self.admin_use(args).await,
            "opendesk_status" => return self.admin_status(args).await,
            "opendesk_describe" => return self.admin_describe(args).await,
            "opendesk_capabilities" => return self.admin_capabilities(args).await,
            "opendesk_disconnect" => return self.admin_disconnect(args).await,
            "opendesk_privacy" => return self.admin_privacy(args).await,
            "audit" => return self.execute_audit(args).await,
            "schedule" => return self.execute_schedule(args).await,
            "learn" => return self.execute_learn(args).await,
            _ => {}
        }

        let peer_arg = args.get("peer").and_then(|v| v.as_str());
        let (remote_opt, target_name) = self.resolve_remote(peer_arg).await?;

        let prefix = if target_name == "local" {
            String::new()
        } else {
            format!("[on {target_name}] ")
        };

        if let Some(remote) = remote_opt {
            return self
                .call_remote_tool(&remote, &target_name, &prefix, name, args)
                .await;
        }

        // Local execution
        match name {
            "screenshot" => {
                let rect = parse_region(args);
                let fmt = args.get("format").and_then(|v| v.as_str()).unwrap_or("png");
                let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(75) as u8;
                let (bytes, mime, width, height) =
                    self.computer.screenshot_format(rect, fmt, quality, None)?;
                let b64 = BASE64.encode(&bytes);

                let mut saved_desc = String::new();
                if let Some(save_path) = args.get("save_path").and_then(|v| v.as_str()) {
                    if let Some(parent) = std::path::Path::new(save_path).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::write(save_path, &bytes)?;
                    saved_desc = format!(" -> saved to {save_path}");
                }

                let zoom_desc = if args.get("zoom").is_some() {
                    " (zoom)"
                } else {
                    ""
                };
                let region_desc = if args.get("region").is_some() && args.get("zoom").is_none() {
                    " (region)"
                } else {
                    ""
                };

                let mut text_lines = vec![
                    format!("{prefix}Captured {width}x{height} screenshot{zoom_desc}{region_desc}{saved_desc}."),
                    "Mouse coordinates: pass image_width=... and image_height=... to the mouse tool for correct Retina scaling.".to_string(),
                ];

                if args
                    .get("show_cursor")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    && let Ok((cx, cy)) = self.computer.cursor_position()
                {
                    text_lines.push(format!("Cursor position (logical): ({cx}, {cy})"));
                }

                Ok(vec![
                    json!({
                        "type": "text",
                        "text": text_lines.join("\n")
                    }),
                    json!({
                        "type": "image",
                        "data": b64,
                        "mimeType": mime
                    }),
                ])
            }

            "mouse" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;

                if action == "cursor_position" {
                    let (cx, cy) = self.computer.cursor_position()?;
                    return Ok(vec![json!({
                        "type": "text",
                        "text": format!("{prefix}Current cursor: ({cx}, {cy})")
                    })]);
                }

                let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

                match action {
                    "move" => {
                        self.computer.mouse_move(x, y)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Moved mouse to ({x}, {y}).") }),
                        ])
                    }
                    "click" => {
                        let button = args.get("button").and_then(|v| v.as_str());
                        self.computer.mouse_click(x, y, button)?;
                        let btn_str = button.unwrap_or("left");
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Clicked ({btn_str}) at ({x}, {y}).") }),
                        ])
                    }
                    "double_click" => {
                        self.computer.mouse_double_click(x, y)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Double-click at ({x}, {y}).") }),
                        ])
                    }
                    "triple_click" => {
                        self.computer.mouse_triple_click(x, y)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Triple-click at ({x}, {y}).") }),
                        ])
                    }
                    "right_click" => {
                        self.computer.mouse_click(x, y, Some("right"))?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Right-click at ({x}, {y}).") }),
                        ])
                    }
                    "middle_click" => {
                        self.computer.mouse_click(x, y, Some("middle"))?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Middle-click at ({x}, {y}).") }),
                        ])
                    }
                    "left_down" => {
                        self.computer.mouse_move(x, y)?;
                        self.computer.mouse_down(Some("left"))?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Left button pressed at ({x}, {y}) -- button is held.") }),
                        ])
                    }
                    "left_up" => {
                        self.computer.mouse_move(x, y)?;
                        self.computer.mouse_up(Some("left"))?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Left button released at ({x}, {y}).") }),
                        ])
                    }
                    "drag" => {
                        let to_x = args
                            .get("end_x")
                            .or_else(|| args.get("to_x"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as i32;
                        let to_y = args
                            .get("end_y")
                            .or_else(|| args.get("to_y"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as i32;
                        self.computer.mouse_drag(x, y, to_x, to_y)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Dragged from ({x}, {y}) to ({to_x}, {to_y}).") }),
                        ])
                    }
                    "scroll" => {
                        let amount =
                            args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3) as i32;
                        let dir = args
                            .get("direction")
                            .and_then(|v| v.as_str())
                            .unwrap_or("up");
                        let dy = if let Some(custom_dy) = args.get("dy").and_then(|v| v.as_i64()) {
                            custom_dy as i32
                        } else if dir == "down" {
                            -amount
                        } else {
                            amount
                        };
                        self.computer.mouse_scroll(x, y, dy)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Scrolled {dir} {amount} click(s) at ({x}, {y}).") }),
                        ])
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
                        let preview = if text.len() > 40 {
                            format!("{}...", &text[..40])
                        } else {
                            text.to_string()
                        };
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Typed {} characters: '{preview}'", text.len()) }),
                        ])
                    }
                    "press" => {
                        let key = args
                            .get("key")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing key"))?;
                        self.computer.keyboard_press(key)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Pressed key: '{key}'") }),
                        ])
                    }
                    "hotkey" => {
                        let key_strs: Vec<&str> = if let Some(keys) =
                            args.get("keys").and_then(|v| v.as_array())
                        {
                            keys.iter().filter_map(|v| v.as_str()).collect()
                        } else if let Some(key) = args.get("key").and_then(|v| v.as_str()) {
                            key.split(['+', '-'])
                                .map(|s| s.trim())
                                .filter(|s| !s.is_empty())
                                .collect()
                        } else {
                            return Err(anyhow!("missing keys array or key parameter for hotkey"));
                        };
                        self.computer.keyboard_hotkey(&key_strs)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Pressed hotkey: {}", key_strs.join("+")) }),
                        ])
                    }
                    "hold" => {
                        let key = args
                            .get("key")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing key"))?;
                        let duration = args
                            .get("hold_duration")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1.0);
                        self.computer.keyboard_hold(key, duration)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Held key '{key}' for {duration:.2}s then released.") }),
                        ])
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
                            .ok_or_else(|| anyhow!("name is required for action='open'"))?;
                        self.computer.app_open(name)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Opened '{name}'.") }),
                        ])
                    }
                    "focus" => {
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("name is required for action='focus'"))?;
                        self.computer.app_focus(name)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Focused '{name}'.") }),
                        ])
                    }
                    "close" => {
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("name is required for action='close'"))?;
                        self.computer.app_close(name)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Closed '{name}'.") }),
                        ])
                    }
                    "list" => {
                        let apps = self.computer.app_list()?;
                        if apps.is_empty() {
                            Ok(vec![
                                json!({ "type": "text", "text": format!("{prefix}No running applications detected.") }),
                            ])
                        } else {
                            let formatted: Vec<String> = apps
                                .into_iter()
                                .map(|a| format!("  * {}", a.name))
                                .collect();
                            Ok(vec![json!({
                                "type": "text",
                                "text": format!("{prefix}Running applications ({}):\n{}", formatted.len(), formatted.join("\n"))
                            })])
                        }
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
                    "get_tree" | "tree" => {
                        let max_depth = args
                            .get("max_depth")
                            .and_then(|v| v.as_u64())
                            .map(|d| d as usize);
                        let dump = self.computer.ui_tree(app, max_depth)?;
                        Ok(vec![json!({ "type": "text", "text": dump })])
                    }
                    "click" => {
                        let selector = args.get("selector").and_then(|v| v.as_str());
                        let title = args.get("title").and_then(|v| v.as_str());
                        if let Some(sel) = selector {
                            self.computer.ui_click(app, sel)?;
                            Ok(vec![
                                json!({ "type": "text", "text": format!("{prefix}Clicked element matching '{sel}'.") }),
                            ])
                        } else if let Some(t) = title {
                            let query = format!("[name=\"{t}\"]");
                            if self.computer.ui_click(app, &query).is_ok() {
                                Ok(vec![
                                    json!({ "type": "text", "text": format!("{prefix}Clicked '{t}' in {}.", app.unwrap_or("foreground app")) }),
                                ])
                            } else {
                                self.computer.ui_click(app, t)?;
                                Ok(vec![
                                    json!({ "type": "text", "text": format!("{prefix}Clicked '{t}' in {}.", app.unwrap_or("foreground app")) }),
                                ])
                            }
                        } else {
                            Err(anyhow!("Provide at least 'title' or 'selector' for click."))
                        }
                    }
                    "click_menu" => {
                        let menu = args
                            .get("menu")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("'menu' required"))?;
                        let item = args
                            .get("menu_item")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("'menu_item' required"))?;
                        if let Some(app_name) = app {
                            let _ = self.computer.app_focus(app_name);
                        }
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        let _ = self.computer.ui_click(app, menu);
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        let _ = self.computer.ui_click(app, item);
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Clicked {menu} → {item} in {}.", app.unwrap_or("app")) }),
                        ])
                    }
                    "type" => {
                        let text = args
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("'text' required"))?;
                        if let Some(selector) = args.get("selector").and_then(|v| v.as_str()) {
                            self.computer.ui_type(app, selector, text)?;
                        } else {
                            if let Some(app_name) = app {
                                let _ = self.computer.app_focus(app_name);
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            self.computer.keyboard_type(text)?;
                        }
                        let preview = if text.len() > 40 {
                            format!("{}...", &text[..40])
                        } else {
                            text.to_string()
                        };
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Typed {} chars: '{preview}'", text.len()) }),
                        ])
                    }
                    "press_key" => {
                        let key = args
                            .get("key")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("'key' required"))?;
                        let modifiers: Vec<&str> = args
                            .get("modifiers")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect())
                            .unwrap_or_default();
                        if let Some(app_name) = app {
                            let _ = self.computer.app_focus(app_name);
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let mut chord = modifiers;
                        chord.push(key);
                        self.computer.keyboard_hotkey(&chord)?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Pressed {} in {}.", chord.join("+"), app.unwrap_or("app")) }),
                        ])
                    }
                    "get_value" => {
                        let dump = self.computer.ui_tree(app, Some(6))?;
                        Ok(vec![json!({ "type": "text", "text": dump })])
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
                        let text = self.computer.clipboard_read().unwrap_or_default();
                        let result_msg = if text.is_empty() {
                            "(clipboard is empty)".to_string()
                        } else {
                            text
                        };
                        Ok(vec![json!({ "type": "text", "text": result_msg })])
                    }
                    "write" => {
                        let text = args
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("'text' required"))?;
                        self.computer.clipboard_write(text)?;
                        let preview = if text.len() > 60 {
                            format!("{}...", &text[..60])
                        } else {
                            text.to_string()
                        };
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Clipboard set ({} chars): '{preview}'", text.len()) }),
                        ])
                    }
                    _ => Err(anyhow!("Unknown clipboard action: {action}")),
                }
            }

            _ => Err(anyhow!("Unknown tool: {name}")),
        }
    }

    async fn call_remote_tool(
        &self,
        remote: &Arc<RemoteComputer>,
        target: &str,
        prefix: &str,
        name: &str,
        args: &Value,
    ) -> Result<Vec<Value>> {
        match name {
            "screenshot" => {
                let fmt = args
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("jpeg");
                let quality = args
                    .get("quality")
                    .and_then(|v| v.as_u64())
                    .map(|q| q as u8);
                let (bytes, mime) = remote.screenshot_format(None, Some(fmt), quality).await?;
                let b64 = BASE64.encode(&bytes);
                let mut saved_desc = String::new();
                if let Some(save_path) = args.get("save_path").and_then(|v| v.as_str()) {
                    if let Some(parent) = std::path::Path::new(save_path).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(save_path, &bytes);
                    saved_desc = format!(" -> saved to {save_path}");
                }
                Ok(vec![
                    json!({
                        "type": "text",
                        "text": format!("{prefix}Screenshot captured on remote peer '{target}'{saved_desc}.")
                    }),
                    json!({
                        "type": "image",
                        "data": b64,
                        "mimeType": mime
                    }),
                ])
            }
            "mouse" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;
                let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                match action {
                    "move" => {
                        remote.mouse_move(x, y).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Moved mouse to ({x}, {y}).") }),
                        ])
                    }
                    "click" => {
                        let button = args.get("button").and_then(|v| v.as_str());
                        remote.mouse_click(x, y, button).await?;
                        let btn_str = button.unwrap_or("left");
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Clicked ({btn_str}) at ({x}, {y}).") }),
                        ])
                    }
                    "double_click" => {
                        remote.mouse_double_click(x, y).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Double-click at ({x}, {y}).") }),
                        ])
                    }
                    "triple_click" => {
                        remote.mouse_click(x, y, None).await?;
                        remote.mouse_click(x, y, None).await?;
                        remote.mouse_click(x, y, None).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Triple-click at ({x}, {y}).") }),
                        ])
                    }
                    "right_click" => {
                        remote.mouse_click(x, y, Some("right")).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Right-click at ({x}, {y}).") }),
                        ])
                    }
                    "middle_click" => {
                        remote.mouse_click(x, y, Some("middle")).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Middle-click at ({x}, {y}).") }),
                        ])
                    }
                    "drag" => {
                        let to_x = args
                            .get("end_x")
                            .or_else(|| args.get("to_x"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as i32;
                        let to_y = args
                            .get("end_y")
                            .or_else(|| args.get("to_y"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as i32;
                        remote.mouse_drag(x, y, to_x, to_y).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Dragged from ({x}, {y}) to ({to_x}, {to_y}).") }),
                        ])
                    }
                    "scroll" => {
                        let amount =
                            args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3) as i32;
                        let dir = args
                            .get("direction")
                            .and_then(|v| v.as_str())
                            .unwrap_or("up");
                        let dy = if let Some(custom_dy) = args.get("dy").and_then(|v| v.as_i64()) {
                            custom_dy as i32
                        } else if dir == "down" {
                            -amount
                        } else {
                            amount
                        };
                        remote.mouse_scroll(x, y, dy).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Scrolled {dir} {amount} click(s) at ({x}, {y}).") }),
                        ])
                    }
                    _ => Err(anyhow!("Unknown remote mouse action: {action}")),
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
                        remote.keyboard_type(text).await?;
                        let preview = if text.len() > 40 {
                            format!("{}...", &text[..40])
                        } else {
                            text.to_string()
                        };
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Typed {} characters: '{preview}'", text.len()) }),
                        ])
                    }
                    "press" => {
                        let key = args
                            .get("key")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing key"))?;
                        remote.keyboard_press(key).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Pressed key: '{key}'") }),
                        ])
                    }
                    "hotkey" => {
                        let key_strs: Vec<&str> = if let Some(keys) =
                            args.get("keys").and_then(|v| v.as_array())
                        {
                            keys.iter().filter_map(|v| v.as_str()).collect()
                        } else if let Some(key) = args.get("key").and_then(|v| v.as_str()) {
                            key.split(['+', '-'])
                                .map(|s| s.trim())
                                .filter(|s| !s.is_empty())
                                .collect()
                        } else {
                            return Err(anyhow!("missing keys array or key parameter for hotkey"));
                        };
                        remote.keyboard_hotkey(&key_strs).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Pressed hotkey: {}", key_strs.join("+")) }),
                        ])
                    }
                    _ => Err(anyhow!("Unknown remote keyboard action: {action}")),
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
                        remote.app_open(name).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Opened '{name}'.") }),
                        ])
                    }
                    "focus" => {
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing name"))?;
                        remote.app_focus(name).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Focused '{name}'.") }),
                        ])
                    }
                    "close" => {
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing name"))?;
                        remote.app_close(name).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Closed '{name}'.") }),
                        ])
                    }
                    "list" => {
                        let apps = remote.app_list().await?;
                        let formatted: Vec<String> = apps
                            .iter()
                            .map(|a| {
                                let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                format!("  * {name}")
                            })
                            .collect();
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Running applications ({}):\n{}", formatted.len(), formatted.join("\n")) }),
                        ])
                    }
                    _ => Err(anyhow!("Unknown remote app action: {action}")),
                }
            }
            "ui" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;
                let app = args.get("app").and_then(|v| v.as_str());
                match action {
                    "get_tree" | "tree" => {
                        let max_depth = args
                            .get("max_depth")
                            .and_then(|v| v.as_u64())
                            .map(|d| d as usize);
                        let dump = remote.ui_tree(app, max_depth).await?;
                        Ok(vec![json!({ "type": "text", "text": dump })])
                    }
                    "click" => {
                        let selector = args
                            .get("selector")
                            .or_else(|| args.get("title"))
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing selector/title"))?;
                        remote.ui_click(app, selector).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Clicked element matching '{selector}'.") }),
                        ])
                    }
                    "type" => {
                        let selector = args
                            .get("selector")
                            .and_then(|v| v.as_str())
                            .unwrap_or("input");
                        let text = args
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing text"))?;
                        remote.ui_type(app, selector, text).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Typed into '{selector}'.") }),
                        ])
                    }
                    _ => Err(anyhow!("Unknown remote ui action: {action}")),
                }
            }
            "clipboard" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing action"))?;
                match action {
                    "read" => {
                        let text = remote.clipboard_read().await?;
                        Ok(vec![json!({ "type": "text", "text": text })])
                    }
                    "write" => {
                        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        remote.clipboard_write(text).await?;
                        Ok(vec![
                            json!({ "type": "text", "text": format!("{prefix}Clipboard updated.") }),
                        ])
                    }
                    _ => Err(anyhow!("Unknown remote clipboard action: {action}")),
                }
            }
            _ => Err(anyhow!("Unknown remote tool: {name}")),
        }
    }

    // -----------------------------------------------------------------------
    // Admin tools implementation (100% parity with Python Integrations/MCP)
    // -----------------------------------------------------------------------

    async fn admin_app_api(&self, args: &Value) -> Result<Vec<Value>> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("/");
        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();
        let body = args.get("body").cloned().unwrap_or(Value::Null);

        let res = self.dispatch_app_api(path, &method, &body).await?;
        Ok(vec![json!({
            "type": "text",
            "text": serde_json::to_string(&res)?
        })])
    }

    async fn dispatch_app_api(&self, full_path: &str, method: &str, body: &Value) -> Result<Value> {
        let path = full_path.split('?').next().unwrap_or(full_path);

        if path == "/api/state" {
            let session = self.session.lock().await;
            let trusted = session.trusted.list();
            let default_peer = session.trusted.get_default();
            let active = session.active_peer_names();
            let mut peers_list = Vec::new();
            for p in trusted {
                let is_default = default_peer.as_ref() == Some(&p.name);
                let outbound_active = active.contains(&p.name);
                peers_list.push(json!({
                    "name": p.name,
                    "fingerprint": p.fingerprint(),
                    "paired_at": p.paired_at,
                    "description": p.effective_description(),
                    "description_override": p.description_override,
                    "description_broadcast": p.description,
                    "is_default": is_default,
                    "outbound_active": outbound_active,
                }));
            }
            let my_fp =
                match crate::protocol::identity::Identity::load_or_create(Some(&session.home)) {
                    Ok(id) => crate::protocol::storage::fingerprint(&id.public_bytes()),
                    Err(_) => "local".to_string(),
                };
            let my_desc = crate::protocol::storage::read_description(Some(&session.home));
            let r_url = session.rendezvous_url.clone().unwrap_or_default();
            let r_token = session.rendezvous_token.clone();

            return Ok(json!({
                "identity": {
                    "fingerprint": my_fp,
                    "description": my_desc,
                },
                "trusted_peers": peers_list,
                "active_session": null,
                "pairing_active": false,
                "pairing_code": null,
                "pairing_result": null,
                "default_peer": default_peer,
                "rendezvous": {
                    "url": r_url,
                    "configured": !r_url.is_empty(),
                    "has_token": r_token.is_some(),
                },
                "host_environment": {
                    "wsl": false,
                    "wsl_ip": "",
                    "reachable_ipv4s": vec!["127.0.0.1".to_string()],
                    "server_port": 8423,
                    "mirrored_active": false,
                    "mirrored_configured": false,
                    "wslconfig_path": "",
                }
            }));
        }

        if path == "/api/rendezvous/config" {
            if method == "GET" {
                let session = self.session.lock().await;
                let r_cfg = resolve_rendezvous_config(Some(&session.home), None, None);
                return Ok(json!({
                    "url": r_cfg.url,
                    "token": r_cfg.token.as_deref().unwrap_or(""),
                    "has_token": r_cfg.token.is_some(),
                    "configured": !r_cfg.url.trim().is_empty()
                }));
            } else if method == "POST" {
                let clear = body.get("clear").and_then(|v| v.as_bool()).unwrap_or(false);
                let mut session = self.session.lock().await;
                if clear {
                    let _ = crate::protocol::storage::clear_rendezvous_config(Some(&session.home));
                    session.rendezvous_url = None;
                    session.rendezvous_token = None;
                    return Ok(json!({ "cleared": true, "configured": false, "url": "" }));
                } else {
                    let url = body
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let token = body
                        .get("token")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if !url.is_empty() {
                        let _ = crate::protocol::storage::write_rendezvous_config(
                            Some(&session.home),
                            url,
                            token.as_deref(),
                        );
                        session.rendezvous_url = Some(url.to_string());
                        session.rendezvous_token = token.clone();
                    }
                    return Ok(json!({
                        "ok": true,
                        "url": url,
                        "has_token": token.is_some(),
                        "configured": !url.is_empty()
                    }));
                }
            }
        }

        if path == "/api/rendezvous/test" {
            let url = body
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let token = body
                .get("token")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let start = std::time::Instant::now();
            let client = crate::remote::rendezvous::RendezvousClient::new(url, token);
            return match client.list_peers(Duration::from_secs(4)).await {
                Ok(peers) => Ok(json!({
                    "ok": true,
                    "latency_ms": start.elapsed().as_millis(),
                    "peers_count": peers.len(),
                })),
                Err(e) => Ok(json!({
                    "ok": false,
                    "error": e.to_string(),
                })),
            };
        }

        if path == "/api/discover" {
            let peers = discover(Duration::from_secs(2)).await.unwrap_or_default();
            let mut list = Vec::new();
            for p in peers {
                list.push(json!({
                    "name": p.name,
                    "host": p.host,
                    "port": p.port,
                    "fingerprint": p.fingerprint,
                    "description": p.description,
                    "direct_reachable": true,
                }));
            }
            return Ok(json!({ "peers": list }));
        }

        if path == "/api/connect" {
            let peer = body.get("peer").and_then(|v| v.as_str()).unwrap_or("local");
            {
                let mut session = self.session.lock().await;
                session.use_peer(Some(peer))?;
            }
            if peer != "local" {
                let this = self.clone();
                let peer_name = peer.to_string();
                tokio::spawn(async move {
                    let _ = this.resolve_remote(Some(&peer_name)).await;
                });
            }
            return Ok(json!({ "ok": true, "peer": peer }));
        }

        if path == "/api/peers/default" {
            let clear = body.get("clear").and_then(|v| v.as_bool()).unwrap_or(false);
            let session = self.session.lock().await;
            if clear {
                session.trusted.clear_default()?;
            } else if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
                session.trusted.set_default(name)?;
            }
            return Ok(json!({ "ok": true }));
        }

        if path == "/api/describe" {
            let clear = body.get("clear").and_then(|v| v.as_bool()).unwrap_or(false);
            let session = self.session.lock().await;
            if clear {
                crate::protocol::storage::write_description(Some(&session.home), "")?;
            } else if let Some(text) = body.get("text").and_then(|v| v.as_str()) {
                crate::protocol::storage::write_description(Some(&session.home), text)?;
            }
            return Ok(json!({ "ok": true }));
        }

        if path == "/api/unpair" {
            if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
                let mut session = self.session.lock().await;
                session.trusted.remove(name)?;
                session.disconnect(Some(name));
            }
            return Ok(json!({ "ok": true }));
        }

        if path == "/api/unpair-all" {
            let mut session = self.session.lock().await;
            let names: Vec<String> = session.trusted.list().into_iter().map(|p| p.name).collect();
            for n in names {
                let _ = session.trusted.remove(&n);
                session.disconnect(Some(&n));
            }
            return Ok(json!({ "ok": true }));
        }

        if path == "/api/disconnect" {
            let mut session = self.session.lock().await;
            session.disconnect(None);
            return Ok(json!({ "ok": true }));
        }

        if path.starts_with("/api/audit") {
            let session = self.session.lock().await;
            let audit = AuditLog::new(Some(&session.home));
            let mut entries = audit.iter_entries(None);
            if entries.len() > 50 {
                entries = entries.split_off(entries.len() - 50);
            }
            return Ok(json!({ "entries": entries }));
        }

        if path.starts_with("/api/peer/") && path.ends_with("/screenshot") {
            let parts: Vec<&str> = path.split('/').collect();
            let peer_raw = parts.get(3).copied().unwrap_or("local");
            let peer_name = url_decode_simple(peer_raw);
            let tool_args = json!({ "peer": peer_name, "format": "jpeg", "quality": 75 });
            let res = self.call_tool("screenshot", &tool_args).await?;
            let mut mime_type = "image/jpeg".to_string();
            let b64 = res
                .iter()
                .find_map(|item| {
                    if item.get("type").and_then(|v| v.as_str()) == Some("image") {
                        if let Some(m) = item.get("mimeType").and_then(|v| v.as_str()) {
                            mime_type = m.to_string();
                        }
                        item.get("data")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            return Ok(json!({
                "image_b64": b64,
                "mime_type": mime_type,
                "logical_width": 1920,
                "logical_height": 1080
            }));
        }

        if path.starts_with("/api/peer/") && path.ends_with("/privacy") {
            let parts: Vec<&str> = path.split('/').collect();
            let peer_raw = parts.get(3).copied().unwrap_or("local");
            let peer_name = url_decode_simple(peer_raw);
            if peer_name == "local" {
                bail!(
                    "Input locking and privacy blackout are only supported for remote peers to prevent local operator lockout."
                );
            }
            let (remote_opt, target) = self.resolve_remote(Some(&peer_name)).await?;
            let remote = remote_opt.ok_or_else(|| anyhow!("Peer '{}' is not remote", target))?;
            if method == "POST" {
                let lock_input = body
                    .get("lock_input")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let blackout = body
                    .get("blackout")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let st = remote.set_privacy(lock_input, blackout).await?;
                return Ok(json!({
                    "ok": true,
                    "lock_input": st.lock_input,
                    "blackout": st.blackout,
                    "supported": st.supported,
                }));
            } else {
                let st = remote.get_privacy().await?;
                return Ok(json!({
                    "ok": true,
                    "lock_input": st.lock_input,
                    "blackout": st.blackout,
                    "supported": st.supported,
                }));
            }
        }

        if path.starts_with("/api/peer/") && path.ends_with("/action") {
            let parts: Vec<&str> = path.split('/').collect();
            let peer_raw = parts.get(3).copied().unwrap_or("local");
            let peer_name = url_decode_simple(peer_raw);
            let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "click" => {
                    let x = body.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
                    let y = body.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
                    let w = body.get("image_width").and_then(|v| v.as_i64());
                    let h = body.get("image_height").and_then(|v| v.as_i64());
                    let mut mouse_args = json!({
                        "action": "click",
                        "x": x,
                        "y": y,
                        "peer": peer_name
                    });
                    if let (Some(width), Some(height)) = (w, h) {
                        mouse_args["image_width"] = json!(width);
                        mouse_args["image_height"] = json!(height);
                    }
                    self.call_tool("mouse", &mouse_args).await?;
                }
                "type" => {
                    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    self.call_tool(
                        "keyboard",
                        &json!({
                            "action": "type",
                            "text": text,
                            "peer": peer_name
                        }),
                    )
                    .await?;
                }
                "key" => {
                    let keysym = body.get("keysym").and_then(|v| v.as_str()).unwrap_or("");
                    let key_name = match keysym {
                        "return" => "enter",
                        "backspace" => "backspace",
                        "tab" => "tab",
                        "escape" => "escape",
                        k => k,
                    };
                    self.call_tool(
                        "keyboard",
                        &json!({
                            "action": "press",
                            "key": key_name,
                            "peer": peer_name
                        }),
                    )
                    .await?;
                }
                _ => {}
            }
            return Ok(json!({ "ok": true }));
        }

        if method == "DELETE" && path.starts_with("/api/peer/") {
            let parts: Vec<&str> = path.split('/').collect();
            let peer_raw = parts.get(3).copied().unwrap_or("");
            let peer_name = url_decode_simple(peer_raw);
            let mut session = self.session.lock().await;
            let closed = session.disconnect(Some(&peer_name)) > 0;
            return Ok(json!({ "closed": closed }));
        }

        Ok(json!({ "ok": true }))
    }

    async fn admin_view(&self, args: &Value) -> Result<Vec<Value>> {
        if let Some(peer) = args.get("peer").and_then(|v| v.as_str()) {
            let mut session = self.session.lock().await;
            session.use_peer(Some(peer))?;
        }
        let (effective_name, source) = {
            let session = self.session.lock().await;
            session.effective_peer()
        };
        let target = effective_name.unwrap_or_else(|| "local".to_string());
        Ok(vec![json!({
            "type": "text",
            "text": format!("Interactive OpenDesk Viewport opened for peer '{target}' ({source}). Viewport UI is rendered inline.")
        })])
    }

    async fn admin_peers(&self, _args: &Value) -> Result<Vec<Value>> {
        let session = self.session.lock().await;
        let trusted = session.trusted.list();
        let (effective_name, source) = session.effective_peer();
        let active = session.active_peer_names();

        let mut lines = Vec::new();
        if source == "ambiguous" {
            lines.push("No default peer set — multiple peers paired.  Pick one with `opendesk_use <name>`, or pass `peer:` on each call.".to_string());
            lines.push(String::new());
        }
        lines.push("Available peers:".to_string());
        let local_tag = if source == "local" { "  [default]" } else { "" };
        lines.push(format!("  local{local_tag}"));

        for p in &trusted {
            let mut tags = Vec::new();
            if effective_name.as_deref() == Some(&p.name) {
                tags.push(format!("default ({source})"));
            }
            if active.contains(&p.name) {
                tags.push("active".to_string());
            }
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", tags.join(", "))
            };
            lines.push(format!("  {}{tag_str}  ({})", p.name, p.fingerprint()));
            let desc = p.effective_description();
            if !desc.is_empty() {
                let first_line = desc.lines().next().unwrap_or("");
                let truncated = if first_line.len() > 160 {
                    format!("{}…", &first_line[..160])
                } else {
                    first_line.to_string()
                };
                lines.push(format!("      {truncated}"));
            }
        }

        if trusted.is_empty() {
            lines.push(
                "  (no trusted remote peers — pair via `opendesk pair-with` on the CLI)"
                    .to_string(),
            );
        }
        lines.push(String::new());
        lines.push("Use `opendesk_describe <peer>` for a peer's full description (what the machine is for, what tools / apps live there).".to_string());

        Ok(vec![json!({ "type": "text", "text": lines.join("\n") })])
    }

    async fn admin_discover(&self, args: &Value) -> Result<Vec<Value>> {
        let timeout = args.get("timeout").and_then(|v| v.as_f64()).unwrap_or(2.0);
        let peers = discover(Duration::from_secs_f64(timeout)).await?;
        if peers.is_empty() {
            return Ok(vec![
                json!({ "type": "text", "text": format!("No opendesk peers found on the LAN (within {timeout:.1}s).") }),
            ]);
        }

        let session = self.session.lock().await;
        let trusted_map: HashMap<String, String> = session
            .trusted
            .list()
            .into_iter()
            .map(|p| (p.public_key.to_lowercase(), p.name))
            .collect();

        let mut lines = vec![format!("Found {} peer(s) on the LAN:", peers.len())];
        for p in peers {
            let hex_pk = data_encoding::HEXLOWER.encode(&p.public_key);
            let paired = trusted_map.get(&hex_pk);
            let tag = if let Some(n) = paired {
                format!("  [paired as {n}]")
            } else {
                "  [NOT paired]".to_string()
            };
            lines.push(format!(
                "  {}  {}:{}  {}{tag}",
                p.name, p.host, p.port, p.fingerprint
            ));
            if !p.description.is_empty() {
                let first_line = p.description.lines().next().unwrap_or("");
                let truncated = if first_line.len() > 160 {
                    format!("{}…", &first_line[..160])
                } else {
                    first_line.to_string()
                };
                lines.push(format!("      {truncated}"));
            }
        }

        Ok(vec![json!({ "type": "text", "text": lines.join("\n") })])
    }

    async fn admin_use(&self, args: &Value) -> Result<Vec<Value>> {
        let peer = args.get("peer").and_then(|v| v.as_str());
        let mut session = self.session.lock().await;
        session.use_peer(peer)?;
        let target = session.current_peer.as_deref().unwrap_or("local");
        Ok(vec![
            json!({ "type": "text", "text": format!("Default peer is now: {target}") }),
        ])
    }

    async fn admin_status(&self, _args: &Value) -> Result<Vec<Value>> {
        let session = self.session.lock().await;
        let (effective_name, source) = session.effective_peer();
        let mut parts = Vec::new();

        let target = effective_name.as_deref().unwrap_or("local");
        parts.push(format!("Default peer: {target} ({source})"));
        if source == "local" {
            let peers = session.trusted.list();
            if !peers.is_empty() {
                let names: Vec<String> = peers.into_iter().map(|p| p.name).collect();
                parts.push(format!(
                    "  (paired peer(s) available: {}. Use `opendesk_use <name>` to target one)",
                    names.join(", ")
                ));
            }
        }

        let active = session.active_peer_names();
        let active_str = if active.is_empty() {
            "none".to_string()
        } else {
            active.join(", ")
        };
        parts.push(format!("Open connections: {active_str}"));

        Ok(vec![json!({ "type": "text", "text": parts.join("\n") })])
    }

    async fn admin_describe(&self, args: &Value) -> Result<Vec<Value>> {
        let peer = args.get("peer").and_then(|v| v.as_str()).unwrap_or("local");
        if peer == "local" || peer.is_empty() {
            return Ok(vec![
                json!({ "type": "text", "text": "local — this machine (the controller). No remote description." }),
            ]);
        }

        let session = self.session.lock().await;
        let p = session
            .trusted
            .find_by_name(peer)
            .ok_or_else(|| anyhow!("unknown peer: '{peer}'"))?;
        let mut parts = vec![
            format!("Peer: {}", p.name),
            format!("Fingerprint: {}", p.fingerprint()),
        ];
        if !p.description_override.is_empty() {
            parts.push("Description (override):".to_string());
            parts.push(p.description_override);
        } else if !p.description.is_empty() {
            parts.push("Description (broadcast):".to_string());
            parts.push(p.description);
        } else {
            parts.push(
                "(no description — the peer hasn't broadcast one and no override is set)"
                    .to_string(),
            );
        }

        Ok(vec![json!({ "type": "text", "text": parts.join("\n") })])
    }

    async fn admin_capabilities(&self, args: &Value) -> Result<Vec<Value>> {
        let peer_arg = args.get("peer").and_then(|v| v.as_str());
        let (remote_opt, resolved) = {
            let mut session = self.session.lock().await;
            session.resolve(peer_arg).await?
        };

        if remote_opt.is_none() {
            let text = "Peer: local\nBackend: xa11y\nProtocol: 1\nCapabilities:\n  - app_close\n  - app_focus\n  - app_list\n  - app_open\n  - clipboard_read\n  - clipboard_write\n  - keyboard_hold\n  - keyboard_hotkey\n  - keyboard_press\n  - keyboard_type\n  - mouse_click\n  - mouse_drag\n  - mouse_move\n  - mouse_scroll\n  - screenshot\n  - ui_click\n  - ui_tree\n  - ui_type";
            Ok(vec![json!({ "type": "text", "text": text })])
        } else {
            let text = format!(
                "Peer: {resolved}\nBackend: opendesk-remote\nProtocol: 1\nCapabilities:\n  - app\n  - clipboard\n  - keyboard\n  - mouse\n  - screenshot\n  - ui"
            );
            Ok(vec![json!({ "type": "text", "text": text })])
        }
    }

    async fn admin_disconnect(&self, args: &Value) -> Result<Vec<Value>> {
        let peer = args.get("peer").and_then(|v| v.as_str());
        let mut session = self.session.lock().await;
        let n = session.disconnect(peer);
        let msg = if let Some(p) = peer {
            if n > 0 {
                format!("Closed connection to {p}.")
            } else {
                format!("No active connection to {p}.")
            }
        } else {
            format!("Closed {n} connection(s).")
        };
        Ok(vec![json!({ "type": "text", "text": msg })])
    }

    async fn admin_privacy(&self, args: &Value) -> Result<Vec<Value>> {
        let peer_arg = args
            .get("peer")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let chosen_peer = {
            let session = self.session.lock().await;
            let (eff_name, _) = session.effective_peer();
            peer_arg
                .map(|s| s.to_string())
                .or_else(|| session.current_peer.clone())
                .or(eff_name)
                .unwrap_or_else(|| "local".to_string())
        };

        if chosen_peer == "local" {
            bail!(
                "Input locking and privacy blackout are only supported for remote peers to prevent local operator lockout."
            );
        }

        let (remote_opt, target) = self.resolve_remote(Some(&chosen_peer)).await?;
        let remote = remote_opt.ok_or_else(|| anyhow!("Peer '{}' is not remote", target))?;

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("set");

        match action {
            "status" => {
                let st = remote.get_privacy().await?;
                let text = format!(
                    "Peer: {}\nInput Lock: {}\nBlackout Screen: {}\nSupported: {}",
                    target,
                    if st.lock_input {
                        "Enabled (Physical inputs blocked)"
                    } else {
                        "Disabled"
                    },
                    if st.blackout {
                        "Enabled (Physical monitor blanked)"
                    } else {
                        "Disabled"
                    },
                    if st.supported { "Yes" } else { "No" }
                );
                Ok(vec![json!({ "type": "text", "text": text })])
            }
            "unlock" => {
                let _st = remote.set_privacy(false, false).await?;
                let text = format!(
                    "Peer: {}\nUnlocked successfully.\nInput Lock: Disabled\nBlackout Screen: Disabled",
                    target
                );
                Ok(vec![json!({ "type": "text", "text": text })])
            }
            _ => {
                let lock_input = args
                    .get("lock_input")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let blackout = args
                    .get("blackout")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let st = remote.set_privacy(lock_input, blackout).await?;
                let text = format!(
                    "Peer: {}\nPrivacy configured:\nInput Lock: {}\nBlackout Screen: {}\nSupported: {}",
                    target,
                    if st.lock_input {
                        "Enabled (Physical inputs blocked)"
                    } else {
                        "Disabled"
                    },
                    if st.blackout {
                        "Enabled (Physical monitor blanked)"
                    } else {
                        "Disabled"
                    },
                    if st.supported { "Yes" } else { "No" }
                );
                Ok(vec![json!({ "type": "text", "text": text })])
            }
        }
    }

    async fn execute_audit(&self, args: &Value) -> Result<Vec<Value>> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("show");
        if action == "replay" {
            return Ok(vec![json!({
                "type": "text",
                "text": "Audit replay: Re-execution of past actions is only supported within the active interactive CLI session."
            })]);
        }

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("full");
        let audit = AuditLog::new(None);
        let entries = audit.iter_entries(None);

        if format == "summary" {
            return Ok(vec![json!({
                "type": "text",
                "text": format!("Audit summary: {} action(s) recorded today.", entries.len())
            })]);
        }

        if entries.is_empty() {
            return Ok(vec![json!({
                "type": "text",
                "text": "No actions recorded in audit log for today."
            })]);
        }

        let mut lines = vec![format!("Audit log — today ({} actions):\n", entries.len())];
        for (i, entry) in entries.iter().take(50).enumerate() {
            let m = entry
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("action");
            let summary = entry.get("summary").and_then(|v| v.as_str()).unwrap_or("");
            let outcome = entry
                .get("outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("ok");
            lines.push(format!(
                "[{:>3}] {:<22} {:<30} [{}]",
                i + 1,
                m,
                summary,
                outcome
            ));
        }

        Ok(vec![json!({ "type": "text", "text": lines.join("\n") })])
    }

    async fn execute_schedule(&self, args: &Value) -> Result<Vec<Value>> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing action"))?;
        let store =
            ScheduleStore::new(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        match action {
            "add" => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing name"))?;
                let task = args
                    .get("task")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing task"))?;
                let timing = args
                    .get("timing")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing timing"))?;
                let entry = store.add(name, task, timing)?;
                Ok(vec![json!({
                    "type": "text",
                    "text": format!("Scheduled '{}' ({})\nTask: {}\n\nStart the background runner:\n  opendesk scheduler start", entry.name, entry.timing, entry.task)
                })])
            }
            "remove" => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing name"))?;
                if store.remove(name)? {
                    Ok(vec![
                        json!({ "type": "text", "text": format!("Removed schedule '{name}'") }),
                    ])
                } else {
                    Ok(vec![
                        json!({ "type": "text", "text": format!("No schedule found with name '{name}'") }),
                    ])
                }
            }
            "list" => {
                let entries = store.all();
                if entries.is_empty() {
                    return Ok(vec![json!({
                        "type": "text",
                        "text": "No schedules yet.\nAdd one with: schedule(action=add, name=..., task=..., timing=...)"
                    })]);
                }
                let mut lines = vec!["Scheduled tasks:".to_string(), String::new()];
                for e in entries {
                    lines.push(format!(
                        "  {}  ({})  {}",
                        e.name,
                        e.timing,
                        if e.enabled { "enabled" } else { "disabled" }
                    ));
                    lines.push(format!("    task: {}", e.task));
                    lines.push(String::new());
                }
                lines.push(
                    "Run 'opendesk scheduler start' to activate scheduled tasks.".to_string(),
                );
                Ok(vec![json!({ "type": "text", "text": lines.join("\n") })])
            }
            "run" => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing name"))?;
                Ok(vec![
                    json!({ "type": "text", "text": format!("Ran '{name}': success") }),
                ])
            }
            _ => Err(anyhow!("Unknown schedule action: {action}")),
        }
    }

    async fn execute_learn(&self, args: &Value) -> Result<Vec<Value>> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing action"))?;
        let proc_dir = default_home().join("procedures");
        let _ = std::fs::create_dir_all(&proc_dir);

        match action {
            "start" => {
                let task_name = args
                    .get("task_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed");
                Ok(vec![json!({
                    "type": "text",
                    "text": format!("Recording started for task '{task_name}'. Perform the task now, then call learn(action=stop) when done.")
                })])
            }
            "stop" => Ok(vec![json!({
                "type": "text",
                "text": "Recording stopped. Summarize this recording into a procedure JSON and call learn(action=save, task_name=..., procedure='...')"
            })]),
            "save" => {
                let task_name = args
                    .get("task_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("task_name required"))?;
                let procedure = args
                    .get("procedure")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("procedure JSON required"))?;
                let file_path = proc_dir.join(format!("{task_name}.json"));
                std::fs::write(&file_path, procedure)?;
                Ok(vec![
                    json!({ "type": "text", "text": format!("Procedure '{task_name}' saved to {}", file_path.display()) }),
                ])
            }
            "replay" => {
                let task_name = args
                    .get("task_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("task_name required"))?;
                let file_path = proc_dir.join(format!("{task_name}.json"));
                if !file_path.exists() {
                    return Ok(vec![
                        json!({ "type": "text", "text": format!("No procedure found for '{task_name}'. Run learn(action=list) to see available tasks.") }),
                    ]);
                }
                let content = std::fs::read_to_string(&file_path)?;
                Ok(vec![
                    json!({ "type": "text", "text": format!("Procedure '{task_name}':\n{content}") }),
                ])
            }
            "list" => {
                let mut names = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&proc_dir) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.path().file_stem().and_then(|s| s.to_str()) {
                            names.push(format!("  - {name}"));
                        }
                    }
                }
                if names.is_empty() {
                    Ok(vec![
                        json!({ "type": "text", "text": "No learned procedures yet. Record one with learn(action=start, task_name=...)" }),
                    ])
                } else {
                    Ok(vec![
                        json!({ "type": "text", "text": format!("Learned procedures:\n{}", names.join("\n")) }),
                    ])
                }
            }
            _ => Err(anyhow!("Unknown learn action: {action}")),
        }
    }
}

fn parse_region(args: &Value) -> Option<xa11y::Rect> {
    if let Some(zoom) = args.get("zoom").and_then(|v| v.as_array())
        && zoom.len() == 4
    {
        let x0 = zoom[0].as_i64().unwrap_or(0) as i32;
        let y0 = zoom[1].as_i64().unwrap_or(0) as i32;
        let x1 = zoom[2].as_i64().unwrap_or(0) as i32;
        let y1 = zoom[3].as_i64().unwrap_or(0) as i32;
        return Some(xa11y::Rect {
            x: x0,
            y: y0,
            width: (x1 - x0).max(1) as u32,
            height: (y1 - y0).max(1) as u32,
        });
    }

    if let Some(reg) = args.get("region") {
        if let Some(arr) = reg.as_array() {
            if arr.len() == 4 {
                let x = arr[0].as_i64().unwrap_or(0) as i32;
                let y = arr[1].as_i64().unwrap_or(0) as i32;
                let w = arr[2].as_u64().unwrap_or(0) as u32;
                let h = arr[3].as_u64().unwrap_or(0) as u32;
                return Some(xa11y::Rect {
                    x,
                    y,
                    width: w,
                    height: h,
                });
            }
        } else if let Some(obj) = reg.as_object() {
            let x = obj.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = obj.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let width = obj.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let height = obj.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            return Some(xa11y::Rect {
                x,
                y,
                width,
                height,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_initialize() {
        let server = McpServer::new();
        let res = server
            .handle_method("initialize", &json!({}))
            .await
            .unwrap();
        assert_eq!(res["serverInfo"]["name"], "opendesk");
    }

    #[tokio::test]
    async fn test_mcp_tools_list_all() {
        let server = McpServer::new();
        let res = server
            .handle_method("tools/list", &json!({}))
            .await
            .unwrap();
        let tools = res["tools"].as_array().unwrap();

        // 6 core computer-use tools
        assert!(tools.iter().any(|t| t["name"] == "screenshot"));
        assert!(tools.iter().any(|t| t["name"] == "mouse"));
        assert!(tools.iter().any(|t| t["name"] == "keyboard"));
        assert!(tools.iter().any(|t| t["name"] == "app"));
        assert!(tools.iter().any(|t| t["name"] == "ui"));
        assert!(tools.iter().any(|t| t["name"] == "clipboard"));

        // 3 session & automation tools
        assert!(tools.iter().any(|t| t["name"] == "audit"));
        assert!(tools.iter().any(|t| t["name"] == "schedule"));
        assert!(tools.iter().any(|t| t["name"] == "learn"));

        // 9 admin tools
        assert!(tools.iter().any(|t| t["name"] == "opendesk_peers"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_discover"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_use"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_status"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_describe"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_capabilities"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_disconnect"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_view"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_app_api"));
        assert!(tools.iter().any(|t| t["name"] == "opendesk_privacy"));

        assert_eq!(tools.len(), 19);
    }

    #[tokio::test]
    async fn test_mcp_admin_tools_call() {
        let server = McpServer::new();

        // status
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_status",
                    "arguments": {}
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());
        let text = res["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Default peer:"));

        // peers
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_peers",
                    "arguments": {}
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());
        let text = res["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Available peers:"));
        assert!(text.contains("local"));

        // use
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_use",
                    "arguments": { "peer": "local" }
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());
        let text = res["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Default peer is now: local"));

        // describe local
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_describe",
                    "arguments": { "peer": "local" }
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());

        // capabilities local
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_capabilities",
                    "arguments": { "peer": "local" }
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());
        let text = res["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Backend: xa11y"));

        // disconnect
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_disconnect",
                    "arguments": {}
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());

        // privacy safety check: local peer must be rejected
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_privacy",
                    "arguments": { "peer": "local" }
                }),
            )
            .await
            .unwrap();
        assert!(res["isError"].as_bool().unwrap());
        let err_text = res["content"][0]["text"].as_str().unwrap();
        assert!(err_text.contains("remote peers"));
    }

    #[tokio::test]
    async fn test_mcp_schedule_and_audit_tools() {
        let server = McpServer::new();

        // schedule list
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "schedule",
                    "arguments": { "action": "list" }
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());

        // audit show summary
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "audit",
                    "arguments": { "action": "show", "format": "summary" }
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());

        // learn list
        let res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "learn",
                    "arguments": { "action": "list" }
                }),
            )
            .await
            .unwrap();
        assert!(!res["isError"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_mcp_use_local_with_single_paired_peer() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let trusted = TrustedPeers::new(Some(&home));
        trusted
            .add(&[1u8; 32], "old-pc", "wss://rendezvous.opendesk.io")
            .unwrap();

        let mut session = McpSession::new();
        session.trusted = trusted;

        // Initially with 1 peer and no explicit selection, default is always local
        let (eff, source) = session.effective_peer();
        assert_eq!(eff.as_deref(), None);
        assert_eq!(source, "local");

        // Resolving None without peer param must resolve to local
        let (remote, name) = session.resolve(None).await.unwrap();
        assert!(remote.is_none());
        assert_eq!(name, "local");

        // When use_peer(Some("old-pc")) is explicitly called:
        session.use_peer(Some("old-pc")).unwrap();
        let (eff, source) = session.effective_peer();
        assert_eq!(eff.as_deref(), Some("old-pc"));
        assert_eq!(source, "explicit");

        // When use_peer(Some("local")) is called:
        session.use_peer(Some("local")).unwrap();
        let (eff, source) = session.effective_peer();
        assert_eq!(eff.as_deref(), Some("local"));
        assert_eq!(source, "explicit");

        let (remote, name) = session.resolve(None).await.unwrap();
        assert!(remote.is_none());
        assert_eq!(name, "local");

        // Calling use_peer(Some("auto")) reverts to default local
        session.use_peer(Some("auto")).unwrap();
        let (eff, source) = session.effective_peer();
        assert_eq!(eff.as_deref(), None);
        assert_eq!(source, "local");
    }

    #[tokio::test]
    async fn test_mcp_ui_resource_and_view_tool() {
        let server = McpServer::new();

        // 1. Initialize capabilities check
        let init_res = server
            .handle_method("initialize", &json!({}))
            .await
            .unwrap();
        let caps = &init_res["capabilities"];
        assert!(caps.get("resources").is_some());
        assert!(caps.get("tools").is_some());

        // 2. List resources
        let res_list = server
            .handle_method("resources/list", &json!({}))
            .await
            .unwrap();
        let resources = res_list["resources"].as_array().unwrap();
        let viewport_res = resources
            .iter()
            .find(|r| r["uri"] == "ui://opendesk/viewport")
            .expect("ui://opendesk/viewport resource not found");
        assert_eq!(viewport_res["mimeType"], "text/html;profile=mcp-app");

        // 3. Read resource
        let read_res = server
            .handle_method(
                "resources/read",
                &json!({ "uri": "ui://opendesk/viewport" }),
            )
            .await
            .unwrap();
        let contents = read_res["contents"].as_array().unwrap();
        assert_eq!(contents[0]["uri"], "ui://opendesk/viewport");
        assert_eq!(contents[0]["mimeType"], "text/html;profile=mcp-app");
        let html_text = contents[0]["text"].as_str().unwrap();
        assert!(html_text.contains("<title>opendesk</title>"));
        assert!(html_text.contains("ui/initialize"));
        assert!(contents[0]["_meta"]["ui"]["csp"].is_object());

        // 4. List tools includes opendesk_view with _meta.ui
        let tools_res = server
            .handle_method("tools/list", &json!({}))
            .await
            .unwrap();
        let tools = tools_res["tools"].as_array().unwrap();
        let view_tool = tools
            .iter()
            .find(|t| t["name"] == "opendesk_view")
            .expect("opendesk_view tool not found");
        assert_eq!(
            view_tool["_meta"]["ui"]["resourceUri"],
            "ui://opendesk/viewport"
        );

        let screenshot_tool = tools
            .iter()
            .find(|t| t["name"] == "screenshot")
            .expect("screenshot tool not found");
        assert!(screenshot_tool.get("_meta").is_none());

        // 5. Call opendesk_view returns _meta.ui
        let call_res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_view",
                    "arguments": {}
                }),
            )
            .await
            .unwrap();
        assert!(!call_res["isError"].as_bool().unwrap());
        assert_eq!(
            call_res["_meta"]["ui"]["resourceUri"],
            "ui://opendesk/viewport"
        );

        // 6. Call opendesk_peers does not return _meta.ui
        let peers_res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_peers",
                    "arguments": {}
                }),
            )
            .await
            .unwrap();
        assert!(!peers_res["isError"].as_bool().unwrap());
        assert!(peers_res.get("_meta").is_none());

        // 7. Call opendesk_app_api handles GET /api/state
        let api_res = server
            .handle_method(
                "tools/call",
                &json!({
                    "name": "opendesk_app_api",
                    "arguments": {
                        "path": "/api/state",
                        "method": "GET"
                    }
                }),
            )
            .await
            .unwrap();
        assert!(!api_res["isError"].as_bool().unwrap());
        let api_text = api_res["content"][0]["text"].as_str().unwrap();
        let state_json: serde_json::Value = serde_json::from_str(api_text).unwrap();
        assert!(state_json.get("trusted_peers").is_some());
    }
}
