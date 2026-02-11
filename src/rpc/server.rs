use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::download_core::engine::{DownloadEngine, DownloadOptions};
use crate::download_core::task::TaskStatus;

/// Shared application state for the RPC server
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<DownloadEngine>,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: String,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl RpcResponse {
    fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: serde_json::Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError { code, message }),
        }
    }
}

async fn handle_rpc(
    State(state): State<AppState>,
    Json(req): Json<RpcRequest>,
) -> Json<RpcResponse> {
    // Token authentication: check first param "token:SECRET" (aria2-style)
    if let Some(ref secret) = state.secret {
        let expected = format!("token:{}", secret);
        let token_ok = req
            .params
            .get(0)
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == expected);
        if !token_ok {
            return Json(RpcResponse::error(
                req.id,
                -32600,
                "Unauthorized: invalid or missing token".into(),
            ));
        }
    }

    let result = dispatch(
        &state.engine,
        &req.method,
        &req.params,
        state.secret.is_some(),
    )
    .await;
    match result {
        Ok(val) => Json(RpcResponse::success(req.id, val)),
        Err(e) => Json(RpcResponse::error(req.id, -32000, e.to_string())),
    }
}

/// When secret is set, params[0] is the token, so real params shift by 1
fn param_offset(has_secret: bool) -> usize {
    if has_secret { 1 } else { 0 }
}

/// Parsed parameters for the `addUri` RPC method.
struct AddUriParams {
    url: String,
    out: Option<String>,
    split: u8,
    extra_headers: Vec<String>,
    extra_cookie: Option<String>,
}

/// Parse addUri parameters from JSON-RPC params.
/// `off` is the param offset (1 when token auth is used, 0 otherwise).
/// `default_split` is the fallback value for the `split` option.
fn parse_adduri_params(
    params: &serde_json::Value,
    off: usize,
    default_split: u8,
) -> anyhow::Result<AddUriParams> {
    let url = params
        .get(off)
        .or_else(|| params.get("url"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing url parameter"))?;

    // Second param can be a string (filename) or an options object
    let second = params.get(off + 1);
    let (out, options) = match second {
        Some(v) if v.is_string() => (v.as_str().map(String::from), None),
        Some(v) if v.is_object() => (
            v.get("out").and_then(|v| v.as_str()).map(String::from),
            Some(v),
        ),
        _ => (None, None),
    };
    // Also support named params as fallback
    let out = out.or_else(|| params.get("out").and_then(|v| v.as_str()).map(String::from));

    let split = options
        .and_then(|o| o.get("split"))
        .or_else(|| params.get("split"))
        .and_then(|v| v.as_u64())
        .unwrap_or(default_split as u64) as u8;

    let extra_headers: Vec<String> = options
        .and_then(|o| o.get("headers"))
        .or_else(|| params.get("headers"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let extra_cookie = options
        .and_then(|o| o.get("cookie"))
        .or_else(|| params.get("cookie"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(AddUriParams {
        url: url.to_string(),
        out,
        split,
        extra_headers,
        extra_cookie,
    })
}

async fn dispatch(
    engine: &Arc<DownloadEngine>,
    method: &str,
    params: &serde_json::Value,
    has_secret: bool,
) -> anyhow::Result<serde_json::Value> {
    let off = param_offset(has_secret);

    match method {
        "addUri" => {
            let default_conn = engine.default_connections();
            let parsed = parse_adduri_params(params, off, default_conn)?;

            let task_id = engine
                .download_background(DownloadOptions {
                    url: parsed.url,
                    output: parsed.out,
                    dir: None,
                    split: parsed.split,
                    max_connections: default_conn,
                    extra_headers: parsed.extra_headers,
                    extra_cookie: parsed.extra_cookie,
                })
                .await?;

            Ok(serde_json::json!(task_id))
        }

        "pause" => {
            let task_id = extract_task_id(params, off)?;
            engine.pause_task(task_id).await?;
            Ok(serde_json::json!(task_id))
        }

        "unpause" => {
            let task_id = extract_task_id(params, off)?;
            engine.resume_task(task_id).await?;
            Ok(serde_json::json!(task_id))
        }

        "remove" => {
            let task_id = extract_task_id(params, off)?;
            engine.remove_task(task_id).await?;
            Ok(serde_json::json!(task_id))
        }

        "tellStatus" => {
            let task_id = extract_task_id(params, off)?;
            let task = engine
                .get_task(task_id)
                .await
                .ok_or_else(|| anyhow::anyhow!("task not found"))?;
            Ok(serde_json::to_value(task)?)
        }

        "tellActive" => {
            let tasks = engine.list_tasks().await;
            let active: Vec<_> = tasks
                .into_iter()
                .filter(|t| t.status == TaskStatus::Downloading)
                .collect();
            Ok(serde_json::to_value(active)?)
        }

        "tellWaiting" => {
            let tasks = engine.list_tasks().await;
            let waiting: Vec<_> = tasks
                .into_iter()
                .filter(|t| t.status == TaskStatus::Pending)
                .collect();
            Ok(serde_json::to_value(waiting)?)
        }

        "tellStopped" => {
            let tasks = engine.list_tasks().await;
            let stopped: Vec<_> = tasks
                .into_iter()
                .filter(|t| {
                    matches!(
                        t.status,
                        TaskStatus::Completed | TaskStatus::Error | TaskStatus::Paused
                    )
                })
                .collect();
            Ok(serde_json::to_value(stopped)?)
        }

        "getGlobalStat" => {
            let stats = engine.global_stats().await;
            Ok(serde_json::to_value(stats)?)
        }

        "changeGlobalOption" => {
            if let Some(limit) = params.get("max-overall-download-limit") {
                if let Some(limit_str) = limit.as_str() {
                    let limit_val = crate::config::parse_speed_limit(limit_str)?;
                    engine.set_speed_limit(limit_val);
                } else if let Some(limit_val) = limit.as_u64() {
                    engine.set_speed_limit(limit_val);
                }
            }
            Ok(serde_json::json!("OK"))
        }

        _ => Err(anyhow::anyhow!("unknown method: {}", method)),
    }
}

fn extract_task_id(params: &serde_json::Value, offset: usize) -> anyhow::Result<&str> {
    params
        .get(offset)
        .or_else(|| params.get("gid"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing task id"))
}

// --- WebSocket RPC ---

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut event_rx = state.engine.subscribe();
    let (resp_tx, mut resp_rx) = tokio::sync::mpsc::channel::<String>(32);

    // Write task: forward both engine events and RPC responses to the client
    let mut write_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Ok(e) => {
                            if let Ok(json) = serde_json::to_string(&e)
                                && ws_tx.send(Message::Text(json.into())).await.is_err()
                            {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
                resp = resp_rx.recv() => {
                    match resp {
                        Some(json) => {
                            if ws_tx.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });

    // Read task: parse incoming RPC requests, dispatch, send response via channel
    let engine = state.engine.clone();
    let has_secret = state.secret.is_some();
    let secret = state.secret.clone();
    let mut read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            let text = match msg {
                Message::Text(t) => t,
                Message::Close(_) => break,
                _ => continue,
            };

            let req: RpcRequest = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(e) => {
                    let resp = RpcResponse::error(
                        serde_json::Value::Null,
                        -32700,
                        format!("parse error: {}", e),
                    );
                    if let Ok(json) = serde_json::to_string(&resp) {
                        let _ = resp_tx.send(json).await;
                    }
                    continue;
                }
            };

            // Token auth for WebSocket
            if let Some(ref sec) = secret {
                let expected = format!("token:{}", sec);
                let token_ok = req
                    .params
                    .get(0)
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| t == expected);
                if !token_ok {
                    let resp = RpcResponse::error(req.id, -32600, "Unauthorized".into());
                    if let Ok(json) = serde_json::to_string(&resp) {
                        let _ = resp_tx.send(json).await;
                    }
                    continue;
                }
            }

            let result = dispatch(&engine, &req.method, &req.params, has_secret).await;
            let resp = match result {
                Ok(val) => RpcResponse::success(req.id, val),
                Err(e) => RpcResponse::error(req.id, -32000, e.to_string()),
            };
            if let Ok(json) = serde_json::to_string(&resp) {
                let _ = resp_tx.send(json).await;
            }
        }
    });

    tokio::select! {
        _ = &mut write_task => {
            read_task.abort();
        },
        _ = &mut read_task => {
            write_task.abort();
        },
    }
}

/// Start the JSON-RPC server (HTTP + WebSocket)
pub async fn start_rpc_server(
    engine: Arc<DownloadEngine>,
    addr: &str,
    secret: Option<String>,
) -> anyhow::Result<()> {
    if secret.is_some() {
        info!("RPC authentication enabled");
    }
    let state = AppState { engine, secret };
    let app = Router::new()
        .route("/jsonrpc", post(handle_rpc))
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("JSON-RPC server listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_offset() {
        assert_eq!(param_offset(false), 0);
        assert_eq!(param_offset(true), 1);
    }

    #[test]
    fn test_extract_task_id_positional() {
        let params = serde_json::json!(["task-123"]);
        let id = extract_task_id(&params, 0).unwrap();
        assert_eq!(id, "task-123");
    }

    #[test]
    fn test_extract_task_id_with_offset() {
        let params = serde_json::json!(["token:secret", "task-456"]);
        let id = extract_task_id(&params, 1).unwrap();
        assert_eq!(id, "task-456");
    }

    #[test]
    fn test_extract_task_id_named() {
        let params = serde_json::json!({"gid": "task-789"});
        let id = extract_task_id(&params, 0).unwrap();
        assert_eq!(id, "task-789");
    }

    #[test]
    fn test_extract_task_id_missing() {
        let params = serde_json::json!([]);
        assert!(extract_task_id(&params, 0).is_err());
    }

    #[test]
    fn test_adduri_options_parsing_string_out() {
        let params = serde_json::json!(["http://example.com/file.zip", "output.zip"]);
        let parsed = parse_adduri_params(&params, 0, 8).unwrap();
        assert_eq!(parsed.url, "http://example.com/file.zip");
        assert_eq!(parsed.out.as_deref(), Some("output.zip"));
        assert_eq!(parsed.split, 8);
        assert!(parsed.extra_headers.is_empty());
        assert!(parsed.extra_cookie.is_none());
    }

    #[test]
    fn test_adduri_options_parsing_object() {
        let params = serde_json::json!([
            "http://example.com/file.zip",
            {
                "out": "renamed.zip",
                "split": 4,
                "headers": ["Referer: https://example.com"],
                "cookie": "sid=abc"
            }
        ]);
        let parsed = parse_adduri_params(&params, 0, 8).unwrap();
        assert_eq!(parsed.url, "http://example.com/file.zip");
        assert_eq!(parsed.out.as_deref(), Some("renamed.zip"));
        assert_eq!(parsed.split, 4);
        assert_eq!(parsed.extra_headers, vec!["Referer: https://example.com"]);
        assert_eq!(parsed.extra_cookie.as_deref(), Some("sid=abc"));
    }

    #[test]
    fn test_adduri_options_no_second_param() {
        let params = serde_json::json!(["http://example.com/file.zip"]);
        let parsed = parse_adduri_params(&params, 0, 8).unwrap();
        assert_eq!(parsed.url, "http://example.com/file.zip");
        assert!(parsed.out.is_none());
        assert_eq!(parsed.split, 8);
        assert!(parsed.extra_headers.is_empty());
        assert!(parsed.extra_cookie.is_none());
    }

    #[test]
    fn test_adduri_with_token_offset() {
        let params =
            serde_json::json!(["token:secret", "http://example.com/file.zip", {"out": "a.zip"}]);
        let parsed = parse_adduri_params(&params, 1, 8).unwrap();
        assert_eq!(parsed.url, "http://example.com/file.zip");
        assert_eq!(parsed.out.as_deref(), Some("a.zip"));
    }

    #[test]
    fn test_adduri_missing_url() {
        let params = serde_json::json!([]);
        assert!(parse_adduri_params(&params, 0, 8).is_err());
    }

    #[test]
    fn test_adduri_named_params() {
        let params =
            serde_json::json!({"url": "http://example.com/f.zip", "out": "o.zip", "split": 2});
        let parsed = parse_adduri_params(&params, 0, 8).unwrap();
        assert_eq!(parsed.url, "http://example.com/f.zip");
        assert_eq!(parsed.out.as_deref(), Some("o.zip"));
        assert_eq!(parsed.split, 2);
    }

    #[test]
    fn test_rpc_response_success() {
        let resp = RpcResponse::success(serde_json::json!(1), serde_json::json!("ok"));
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["result"], "ok");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn test_rpc_response_error() {
        let resp = RpcResponse::error(serde_json::json!(2), -32600, "bad request".into());
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 2);
        assert!(json.get("result").is_none());
        assert_eq!(json["error"]["code"], -32600);
        assert_eq!(json["error"]["message"], "bad request");
    }
}
