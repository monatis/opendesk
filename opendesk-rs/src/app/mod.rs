//! Web application backend for the `opendesk app` local UI.
//!
//! Provides embedded web UI and REST API for controlling and monitoring OpenDesk.

use anyhow::{Context, Result, anyhow};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::protocol::identity::generate_pairing_code;
use crate::protocol::storage::{
    GlobalRendezvousConfig, fingerprint, read_description, resolve_rendezvous_config,
    write_description,
};
use crate::remote::client::{RemoteComputer, connect as client_connect, pair_with};
use crate::remote::discovery::discover;
use crate::remote::server::OpendeskServer;

const INDEX_HTML: &str = include_str!("../../static/index.html");
const STYLES_CSS: &str = include_str!("../../static/styles.css");
const APP_JS: &str = include_str!("../../static/app.js");

#[derive(Clone)]
pub struct AppState {
    pub home: Option<PathBuf>,
    pub server: Arc<OpendeskServer>,
    pub outbound: Arc<Mutex<HashMap<String, Arc<RemoteComputer>>>>,
    pub pairing_code: Arc<Mutex<Option<String>>>,
    pub pairing_result: Arc<Mutex<Option<Value>>>,
    pub pairing_abort_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    pub rendezvous: Arc<Mutex<GlobalRendezvousConfig>>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/static/styles.css", get(styles_handler))
        .route("/static/app.js", get(app_js_handler))
        .route("/api/state", get(get_state))
        .route(
            "/api/rendezvous/config",
            get(get_rendezvous_config).post(save_rendezvous_config),
        )
        .route("/api/rendezvous/test", post(test_rendezvous_connection))
        .route("/api/pair/begin", post(pair_begin))
        .route("/api/pair/cancel", post(pair_cancel))
        .route("/api/disconnect", post(do_disconnect))
        .route("/api/unpair", post(do_unpair))
        .route("/api/unpair-all", post(do_unpair_all))
        .route("/api/peers/default", post(set_default_peer))
        .route("/api/peers/{name}/description", post(set_peer_description))
        .route("/api/describe", post(set_self_description))
        .route("/api/discover", get(do_discover))
        .route("/api/pair-with", post(do_pair_with))
        .route("/api/connect", post(do_connect))
        .route("/api/peer/{name}", delete(close_outbound))
        .route("/api/peer/{name}/screenshot", get(peer_screenshot))
        .route("/api/peer/{name}/action", post(peer_action))
        .route(
            "/api/peer/{name}/privacy",
            get(get_peer_privacy).post(set_peer_privacy),
        )
        .route("/api/audit", get(get_audit))
        .route("/api/wsl/setup", post(wsl_setup))
        .route("/api/wsl/enable-mirrored", post(wsl_enable_mirrored))
        .route("/api/wsl/undo", post(wsl_undo))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Static file handlers
// ---------------------------------------------------------------------------

async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn styles_handler() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], STYLES_CSS)
}

async fn app_js_handler() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], APP_JS)
}

// ---------------------------------------------------------------------------
// REST API handlers
// ---------------------------------------------------------------------------

async fn get_state(State(state): State<AppState>) -> Json<Value> {
    let identity = state.server.identity();
    let trusted = state.server.trusted();
    let my_fp = fingerprint(&identity.public_bytes());

    let outbound_map = state.outbound.lock().await;
    let mut peers_list = Vec::new();
    let default_peer = trusted.get_default();

    for p in trusted.list() {
        let is_default = default_peer.as_ref() == Some(&p.name);
        let outbound_active = outbound_map.contains_key(&p.name);
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

    let active_sessions = state.server.registry().list().await;
    let active_session = if let Some(s) = active_sessions.first() {
        let pk_bytes = data_encoding::HEXLOWER
            .decode(s.peer_pubkey_hex.as_bytes())
            .unwrap_or_default();
        Some(json!({
            "id": s.id,
            "peer_name": s.peer_name,
            "peer_fingerprint": fingerprint(&pk_bytes),
            "remote_addr": s.remote_addr,
            "started_at": s.started_at,
            "age_seconds": s.age_seconds,
            "mode": s.mode,
        }))
    } else {
        None
    };

    let p_code = state.pairing_code.lock().await.clone();
    let mut p_res = state.pairing_result.lock().await;
    let pairing_result = p_res.take();

    let local_ips = get_local_ips();

    let r_cfg = state.rendezvous.lock().await.clone();

    Json(json!({
        "identity": {
            "fingerprint": my_fp,
            "description": read_description(state.home.as_deref()),
        },
        "trusted_peers": peers_list,
        "active_session": active_session,
        "pairing_active": p_code.is_some(),
        "pairing_code": p_code,
        "pairing_result": pairing_result,
        "default_peer": default_peer,
        "rendezvous": {
            "url": r_cfg.url,
            "configured": !r_cfg.url.is_empty(),
            "has_token": r_cfg.token.is_some(),
        },
        "host_environment": {
            "wsl": false,
            "wsl_ip": "",
            "reachable_ipv4s": local_ips,
            "server_port": state.server.port(),
            "mirrored_active": false,
            "mirrored_configured": false,
            "wslconfig_path": "",
        }
    }))
}

async fn get_rendezvous_config(State(state): State<AppState>) -> Json<Value> {
    let r = state.rendezvous.lock().await;
    Json(json!({
        "url": r.url,
        "token": r.token.as_deref().unwrap_or(""),
        "has_token": r.token.is_some(),
        "configured": !r.url.trim().is_empty()
    }))
}

async fn save_rendezvous_config(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let clear = body.get("clear").and_then(|v| v.as_bool()).unwrap_or(false);
    if clear {
        crate::protocol::storage::clear_rendezvous_config(state.home.as_deref())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut r = state.rendezvous.lock().await;
        r.url.clear();
        r.token = None;
        return Ok(Json(
            json!({ "cleared": true, "configured": false, "url": "" }),
        ));
    }

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
        crate::protocol::storage::write_rendezvous_config(
            state.home.as_deref(),
            url,
            token.as_deref(),
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut r = state.rendezvous.lock().await;
        r.url = url.to_string();
        r.token = token;
    }

    let r = state.rendezvous.lock().await;
    Ok(Json(json!({
        "ok": true,
        "url": r.url,
        "has_token": r.token.is_some(),
        "configured": !r.url.is_empty()
    })))
}

async fn test_rendezvous_connection(
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let url = body
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if url.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing 'url'".into()));
    }
    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let start = std::time::Instant::now();
    let client = crate::remote::rendezvous::RendezvousClient::new(url, token);
    match client.list_peers(Duration::from_secs(4)).await {
        Ok(peers) => {
            let latency = start.elapsed().as_millis();
            Ok(Json(json!({
                "ok": true,
                "latency_ms": latency,
                "peers_count": peers.len(),
            })))
        }
        Err(e) => Ok(Json(json!({
            "ok": false,
            "error": e.to_string(),
        }))),
    }
}

async fn pair_begin(State(state): State<AppState>) -> Json<Value> {
    let mut code_guard = state.pairing_code.lock().await;
    if let Some(existing) = code_guard.as_ref() {
        return Json(json!({ "code": existing, "already_active": true }));
    }

    let code = generate_pairing_code(6);
    *code_guard = Some(code.clone());
    *state.pairing_result.lock().await = None;

    let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<()>();
    *state.pairing_abort_tx.lock().await = Some(abort_tx);

    let server = state.server.clone();
    let pair_code = code.clone();
    let res_store = state.pairing_result.clone();
    let code_store = state.pairing_code.clone();

    tokio::spawn(async move {
        tokio::select! {
            _ = abort_rx => {
                info!("Pairing cancelled via API");
            }
            res = server.enable_pairing(&pair_code, 300.0) => {
                match res {
                    Ok(Some(pubkey)) => {
                        let peer_entry = server.trusted().find(&pubkey);
                        let name = peer_entry.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                        let fp = peer_entry.as_ref().map(|p| p.fingerprint()).unwrap_or_default();
                        *res_store.lock().await = Some(json!({
                            "ok": true,
                            "peer_name": name,
                            "fingerprint": fp,
                        }));
                    }
                    Ok(None) => {
                        *res_store.lock().await = Some(json!({
                            "ok": false,
                            "reason": "timeout",
                        }));
                    }
                    Err(e) => {
                        *res_store.lock().await = Some(json!({
                            "ok": false,
                            "reason": e.to_string(),
                        }));
                    }
                }
            }
        }
        *code_store.lock().await = None;
    });

    Json(json!({ "code": code, "already_active": false }))
}

async fn pair_cancel(State(state): State<AppState>) -> Json<Value> {
    if let Some(tx) = state.pairing_abort_tx.lock().await.take() {
        let _ = tx.send(());
        *state.pairing_code.lock().await = None;
        *state.pairing_result.lock().await = None;
        Json(json!({ "cancelled": true }))
    } else {
        Json(json!({ "cancelled": false }))
    }
}

async fn do_disconnect(State(state): State<AppState>) -> Json<Value> {
    let killed = state.server.registry().kill_all("web_disconnect").await;
    Json(json!({ "killed": killed }))
}

async fn do_unpair(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let peer = state
        .server
        .trusted()
        .find_by_name(name)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Kill active session if matched
    let active = state.server.registry().list().await;
    for s in active {
        if s.peer_pubkey_hex == peer.public_key {
            state.server.registry().kill(&s.id, "unpaired").await;
            break;
        }
    }

    // Drop outbound connection
    state.outbound.lock().await.remove(name);

    let ok = state
        .server
        .trusted()
        .remove(name)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "ok": ok })))
}

async fn do_unpair_all(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    let mut count = 0;
    for p in state.server.trusted().list() {
        if state.server.trusted().remove(&p.name).unwrap_or(false) {
            count += 1;
        }
    }
    state.server.registry().kill_all("unpaired_all").await;
    state.outbound.lock().await.clear();
    Ok(Json(json!({ "unpaired": count })))
}

async fn set_default_peer(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if body.get("clear").and_then(|v| v.as_bool()).unwrap_or(false) {
        let cleared = state
            .server
            .trusted()
            .clear_default()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(Json(json!({ "cleared": cleared, "default": Value::Null })));
    }

    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    if state
        .server
        .trusted()
        .set_default(name)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Ok(Json(json!({ "default": name })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn set_peer_description(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if body.get("clear").and_then(|v| v.as_bool()).unwrap_or(false) {
        let ok = state
            .server
            .trusted()
            .clear_description_override(&name)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(Json(json!({ "ok": ok })));
    }
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let ok = state
        .server
        .trusted()
        .set_description_override(&name, text)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if ok {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn set_self_description(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if body.get("clear").and_then(|v| v.as_bool()).unwrap_or(false) {
        crate::protocol::storage::clear_description(state.home.as_deref())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(Json(json!({ "cleared": true })));
    }
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
    write_description(state.home.as_deref(), text)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct DiscoverQuery {
    timeout: Option<f64>,
    rendezvous: Option<String>,
    rendezvous_token: Option<String>,
}

async fn do_discover(
    State(state): State<AppState>,
    Query(q): Query<DiscoverQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let timeout = Duration::from_secs_f64(q.timeout.unwrap_or(2.0));
    let mut list = Vec::new();

    // 1. LAN mDNS discovery
    let own_pk = state.server.identity().public_bytes();
    if let Ok(peers) = discover(timeout).await {
        for p in peers {
            if p.public_key != own_pk {
                list.push(json!({
                    "name": p.name,
                    "host": p.host,
                    "port": p.port,
                    "fingerprint": p.fingerprint,
                    "description": p.description,
                    "public_key_hex": data_encoding::HEXLOWER.encode(&p.public_key),
                    "source": "lan",
                    "rendezvous_url": Value::Null,
                }));
            }
        }
    }

    // 2. Rendezvous discovery
    let (r_url, r_token) = {
        let r = state.rendezvous.lock().await;
        let url = q.rendezvous.clone().or_else(|| {
            if !r.url.is_empty() {
                Some(r.url.clone())
            } else {
                None
            }
        });
        let token = q.rendezvous_token.clone().or_else(|| r.token.clone());
        (url, token)
    };

    if let Some(url) = r_url {
        let r_client = crate::remote::rendezvous::RendezvousClient::new(&url, r_token.as_deref());
        if let Ok(r_peers) = r_client.list_peers(timeout).await {
            for p in r_peers {
                let pk_hex = data_encoding::HEXLOWER.encode(&p.public_key);
                if p.public_key != own_pk {
                    // Check if already in list from LAN
                    if !list.iter().any(|item| {
                        item.get("public_key_hex").and_then(|v| v.as_str()) == Some(&pk_hex)
                    }) {
                        list.push(json!({
                            "name": p.name,
                            "host": "rendezvous",
                            "port": 0,
                            "fingerprint": p.fingerprint,
                            "description": p.description,
                            "public_key_hex": pk_hex,
                            "source": "rendezvous",
                            "rendezvous_url": url,
                        }));
                    }
                }
            }
        }
    }

    Ok(Json(json!({ "peers": list })))
}

async fn do_pair_with(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let host = body.get("host").and_then(|v| v.as_str());
    let code = body
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or((StatusCode::BAD_REQUEST, "missing 'code'".into()))?;
    let name = body.get("name").and_then(|v| v.as_str());
    let desc = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let port = body.get("port").and_then(|v| v.as_u64()).unwrap_or(8423) as u16;
    let target_pubkey = body.get("target_pubkey").and_then(|v| v.as_str());

    let (r_url, r_token) = {
        let r = state.rendezvous.lock().await;
        let url = body
            .get("rendezvous")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                if !r.url.is_empty() {
                    Some(r.url.clone())
                } else {
                    None
                }
            });
        let token = body
            .get("rendezvous_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| r.token.clone());
        (url, token)
    };

    let (remote, server_pub) = pair_with(
        host,
        Some(port),
        code,
        name,
        r_url.as_deref(),
        r_token.as_deref(),
        target_pubkey,
        state.home.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let peer_entry = state.server.trusted().find(&server_pub);
    let peer_name = peer_entry
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| {
            name.map(|n| n.to_string()).unwrap_or_else(|| {
                format!("peer-{}", &data_encoding::HEXLOWER.encode(&server_pub)[..6])
            })
        });

    if !desc.is_empty() {
        let _ = state
            .server
            .trusted()
            .set_description_override(&peer_name, desc);
    }

    let fp = peer_entry
        .as_ref()
        .map(|p| p.fingerprint())
        .unwrap_or_else(|| fingerprint(&server_pub));
    state
        .outbound
        .lock()
        .await
        .insert(peer_name.clone(), Arc::new(remote));

    Ok(Json(json!({
        "ok": true,
        "peer_name": peer_name,
        "fingerprint": fp,
    })))
}

async fn do_connect(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let peer = body
        .get("peer")
        .and_then(|v| v.as_str())
        .ok_or((StatusCode::BAD_REQUEST, "missing 'peer'".into()))?;

    {
        let outbound = state.outbound.lock().await;
        if outbound.contains_key(peer) {
            return Ok(Json(json!({ "ok": true, "reused": true })));
        }
    }

    let (r_url, r_token) = {
        let r = state.rendezvous.lock().await;
        let url = body
            .get("rendezvous")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                if !r.url.is_empty() {
                    Some(r.url.clone())
                } else {
                    None
                }
            });
        let token = body
            .get("rendezvous_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| r.token.clone());
        (url, token)
    };

    let remote = client_connect(
        Some(peer),
        r_url.as_deref(),
        r_token.as_deref(),
        state.home.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    state
        .outbound
        .lock()
        .await
        .insert(peer.to_string(), Arc::new(remote));
    Ok(Json(json!({ "ok": true, "reused": false })))
}

async fn close_outbound(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Json<Value> {
    let removed = state.outbound.lock().await.remove(&name);
    Json(json!({ "closed": removed.is_some() }))
}

async fn peer_screenshot(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Response, (StatusCode, String)> {
    let remote = {
        let outbound = state.outbound.lock().await;
        outbound
            .get(&name)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("not connected: {name}")))?
    };

    let (bytes, mime) = remote
        .screenshot_format(None, Some("jpeg"), Some(75))
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        mime.parse()
            .unwrap_or_else(|_| "image/jpeg".parse().unwrap()),
    );
    headers.insert("X-Logical-Width", "1920".parse().unwrap());
    headers.insert("X-Logical-Height", "1080".parse().unwrap());
    headers.insert("X-Pixel-Width", "1920".parse().unwrap());
    headers.insert("X-Pixel-Height", "1080".parse().unwrap());

    Ok((headers, bytes).into_response())
}

async fn peer_action(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let remote = {
        let outbound = state.outbound.lock().await;
        outbound
            .get(&name)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("not connected: {name}")))?
    };

    let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "click" => {
            let x = body.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
            let y = body.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
            let button = body.get("button").and_then(|v| v.as_str());
            remote
                .mouse_click(x, y, button)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
        }
        "move" => {
            let x = body.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
            let y = body.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
            remote
                .mouse_move(x, y)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
        }
        "scroll" => {
            let x = body.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
            let y = body.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
            let dy = body.get("dy").and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
            remote
                .mouse_scroll(x, y, dy)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
        }
        "type" => {
            let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
            remote
                .keyboard_type(text)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
        }
        "key" => {
            let key = body.get("keysym").and_then(|v| v.as_str()).unwrap_or("");
            remote
                .keyboard_press(key)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unknown action kind: '{kind}'"),
            ));
        }
    }

    Ok(Json(json!({ "ok": true })))
}

async fn get_peer_privacy(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if name == "local" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Input locking and privacy blackout are only supported for remote peers to prevent local operator lockout.".to_string(),
        ));
    }
    let remote = {
        let outbound = state.outbound.lock().await;
        outbound
            .get(&name)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("not connected: {name}")))?
    };

    let st = remote
        .get_privacy()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(json!({
        "ok": true,
        "lock_input": st.lock_input,
        "blackout": st.blackout,
        "supported": st.supported,
    })))
}

async fn set_peer_privacy(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if name == "local" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Input locking and privacy blackout are only supported for remote peers to prevent local operator lockout.".to_string(),
        ));
    }
    let remote = {
        let outbound = state.outbound.lock().await;
        outbound
            .get(&name)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, format!("not connected: {name}")))?
    };

    let lock_input = body
        .get("lock_input")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let blackout = body
        .get("blackout")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let st = remote
        .set_privacy(lock_input, blackout)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(json!({
        "ok": true,
        "lock_input": st.lock_input,
        "blackout": st.blackout,
        "supported": st.supported,
    })))
}

#[derive(Deserialize)]
struct AuditQuery {
    date: Option<String>,
    peer: Option<String>,
    limit: Option<usize>,
}

async fn get_audit(State(state): State<AppState>, Query(q): Query<AuditQuery>) -> Json<Value> {
    let mut entries = state.server.audit().iter_entries(q.date.as_deref());
    if let Some(filter_peer) = q.peer.as_deref() {
        entries.retain(|e| {
            let p_obj = e.get("peer").unwrap_or(&Value::Null);
            let name = p_obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let fp = p_obj.get("fp").and_then(|v| v.as_str()).unwrap_or("");
            name.contains(filter_peer) || fp.contains(filter_peer)
        });
    }

    let limit = q.limit.unwrap_or(200).min(2000);
    if entries.len() > limit {
        entries = entries.split_off(entries.len() - limit);
    }

    Json(json!({ "entries": entries }))
}

async fn wsl_setup() -> Json<Value> {
    Json(json!({ "ok": false, "error": "not running in WSL" }))
}

async fn wsl_enable_mirrored() -> Json<Value> {
    Json(json!({ "ok": false, "error": "not running in WSL" }))
}

async fn wsl_undo() -> Json<Value> {
    Json(json!({ "ok": false, "error": "not running in WSL" }))
}

fn get_local_ips() -> Vec<String> {
    let mut ips = Vec::new();
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0")
        && socket.connect("8.8.8.8:80").is_ok()
        && let Ok(addr) = socket.local_addr()
    {
        let ip = addr.ip().to_string();
        if !ip.starts_with("127.") {
            ips.push(ip);
        }
    }
    if ips.is_empty() {
        ips.push("127.0.0.1".to_string());
    }
    ips
}

pub async fn run_app(
    home: Option<&Path>,
    host: &str,
    port: u16,
    open_browser: bool,
    rendezvous_url: Option<&str>,
    rendezvous_token: Option<&str>,
) -> Result<()> {
    // Check if UI port or WebSocket port is taken
    for (bind_host, bind_port, label) in [("0.0.0.0", 8423, "WebSocket"), (host, port, "UI")] {
        if std::net::TcpListener::bind(format!("{}:{}", bind_host, bind_port)).is_err() {
            return Err(anyhow!(
                "opendesk: {} port {} on {} is already in use.\n  Another opendesk instance is probably running. Stop it and try again.",
                label,
                bind_port,
                bind_host
            ));
        }
    }

    let r_cfg = resolve_rendezvous_config(home, rendezvous_url, rendezvous_token);
    let r_urls = if !r_cfg.url.is_empty() {
        vec![r_cfg.url.clone()]
    } else {
        vec![]
    };
    let r_tok = r_cfg.token.clone();

    let server = Arc::new(OpendeskServer::new("0.0.0.0", 8423, home, r_urls, r_tok)?);

    // Boot background OpendeskServer
    let srv_clone = server.clone();
    tokio::spawn(async move {
        if let Err(e) = srv_clone.serve_forever().await {
            warn!("Background OpendeskServer stopped: {}", e);
        }
    });

    let app_state = AppState {
        home: home.map(|p| p.to_path_buf()),
        server,
        outbound: Arc::new(Mutex::new(HashMap::new())),
        pairing_code: Arc::new(Mutex::new(None)),
        pairing_result: Arc::new(Mutex::new(None)),
        pairing_abort_tx: Arc::new(Mutex::new(None)),
        rendezvous: Arc::new(Mutex::new(r_cfg)),
    };

    let router = create_router(app_state);
    let bind_addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("Failed to bind web UI to {}", bind_addr))?;

    println!("opendesk UI running at http://{}:{}", host, port);

    if open_browser {
        let url = format!("http://{}:{}", host, port);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = webbrowser::open(&url);
        });
    }

    axum::serve(listener, router).await?;
    Ok(())
}
