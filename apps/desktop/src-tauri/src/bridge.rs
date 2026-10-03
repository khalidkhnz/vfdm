//! Loopback HTTP server the browser extension talks to.
//!
//! `GET /ping` is unauthenticated (discovery + badge). `POST /download` needs
//! `Authorization: Bearer <token>`; the token lives in `<app_data>/bridge.token`
//! and the user pastes it into the extension once. Non-extension origins and
//! foreign `Host` headers are rejected to block DNS-rebinding and page scripts.

use crate::events::EV_BRIDGE_REQUEST;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Emitter, Manager};
use vfdm_engine::DownloadRequest;

pub const PROTOCOL_VERSION: u32 = 1;
pub const PORT_RANGE: std::ops::RangeInclusive<u16> = 7800..=7810;
const TOKEN_FILE: &str = "bridge.token";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeInfo {
    pub port: u16,
    pub token: String,
}

pub fn load_or_create_token(data_dir: &Path) -> std::io::Result<String> {
    let path = data_dir.join(TOKEN_FILE);
    match std::fs::read_to_string(&path) {
        Ok(t) if t.trim().len() == 64 => Ok(t.trim().to_string()),
        _ => write_new_token(data_dir),
    }
}

pub fn write_new_token(data_dir: &Path) -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    let path = data_dir.join(TOKEN_FILE);
    std::fs::write(&path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let listener = {
            let mut bound = None;
            for port in PORT_RANGE {
                match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(l) => {
                        bound = Some((port, l));
                        break;
                    }
                    Err(e) => tracing::debug!(port, error = %e, "bridge port busy"),
                }
            }
            match bound {
                Some(b) => b,
                None => {
                    tracing::error!("bridge: no free port in {:?}", PORT_RANGE);
                    return;
                }
            }
        };
        let (port, listener) = listener;
        {
            let state = app.state::<AppState>();
            state.bridge.write().unwrap_or_else(|e| e.into_inner()).port = port;
        }
        tracing::info!(port, "bridge listening");

        let router = Router::new()
            .route("/ping", get(ping))
            .route("/download", post(download))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .layer(middleware::from_fn_with_state(port, guard))
            .with_state(app);
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!(error = %e, "bridge server exited");
        }
    });
}

/// Host must be loopback with our port; Origin must be absent or a browser
/// extension. Answers CORS preflight for allowed origins.
async fn guard(State(port): State<u16>, req: Request<Body>, next: Next) -> Response {
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"))
        .unwrap_or(false);
    if !host_ok {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let origin_ok = match &origin {
        None => true,
        Some(o) => o.starts_with("chrome-extension://") || o.starts_with("moz-extension://"),
    };
    if !origin_ok {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
    }

    let mut resp = if req.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(req).await
    };
    if let Some(o) = origin.and_then(|o| HeaderValue::from_str(&o).ok()) {
        let h = resp.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        h.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
    }
    resp
}

async fn ping(State(app): State<AppHandle>) -> Json<serde_json::Value> {
    let state = app.state::<AppState>();
    let tools = state.tool_status();
    let has = |n: crate::tools::ToolName| tools.iter().any(|t| t.name == n && t.path.is_some());
    Json(serde_json::json!({
        "app": "vfdm",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": PROTOCOL_VERSION,
        "features": ["stream", "ytdlp"],
        "tools": {
            "ffmpeg": has(crate::tools::ToolName::Ffmpeg),
            "ytdlp": has(crate::tools::ToolName::Ytdlp),
        },
    }))
}

fn authorized(state: &AppState, req_headers: &axum::http::HeaderMap) -> bool {
    let Some(v) = req_headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(presented) = v.strip_prefix("Bearer ") else {
        return false;
    };
    let expected = state
        .bridge
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .token
        .clone();
    presented.as_bytes().ct_eq(expected.as_bytes()).into()
}

async fn download(
    State(app): State<AppHandle>,
    Query(q): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<DownloadRequest>,
) -> Response {
    let state = app.state::<AppState>();
    if !authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }
    if q.get("dry").is_some_and(|v| v == "1") {
        return StatusCode::NO_CONTENT.into_response();
    }
    match state.engine.add(req) {
        Ok(id) => {
            if let Some(p) = state.engine.get(id) {
                let _ = app.emit(EV_BRIDGE_REQUEST, &p);
            }
            if state.settings().focus_on_capture {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            Json(serde_json::json!({ "id": id })).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}
