use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::download_core::engine::DownloadEngine;

type SharedEngine = Arc<DownloadEngine>;

#[derive(Debug, Deserialize)]
struct RpcRequest {
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
    State(engine): State<SharedEngine>,
    Json(req): Json<RpcRequest>,
) -> Json<RpcResponse> {
    let result = dispatch(&engine, &req.method, &req.params).await;
    match result {
        Ok(val) => Json(RpcResponse::success(req.id, val)),
        Err(e) => Json(RpcResponse::error(req.id, -32000, e.to_string())),
    }
}

async fn dispatch(
    engine: &DownloadEngine,
    method: &str,
    params: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    match method {
        "addUri" => {
            let url = params
                .get(0)
                .or_else(|| params.get("url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing url parameter"))?;

            let out = params
                .get(1)
                .or_else(|| params.get("out"))
                .and_then(|v| v.as_str())
                .map(String::from);

            let split = params
                .get("split")
                .and_then(|v| v.as_u64())
                .unwrap_or(8) as u8;

            let task_id = engine
                .download(url.to_string(), out, None, split, 8, None)
                .await?;

            Ok(serde_json::json!(task_id))
        }

        "pause" => {
            let task_id = params
                .get(0)
                .or_else(|| params.get("gid"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing task id"))?;

            engine.pause_task(task_id).await?;
            Ok(serde_json::json!(task_id))
        }

        "tellStatus" => {
            let task_id = params
                .get(0)
                .or_else(|| params.get("gid"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing task id"))?;

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
                .filter(|t| t.status == crate::download_core::task::TaskStatus::Downloading)
                .collect();
            Ok(serde_json::to_value(active)?)
        }

        "tellWaiting" => {
            let tasks = engine.list_tasks().await;
            let waiting: Vec<_> = tasks
                .into_iter()
                .filter(|t| t.status == crate::download_core::task::TaskStatus::Pending)
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
                        crate::download_core::task::TaskStatus::Completed
                            | crate::download_core::task::TaskStatus::Error
                            | crate::download_core::task::TaskStatus::Paused
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

/// Start the JSON-RPC server
pub async fn start_rpc_server(engine: Arc<DownloadEngine>, addr: &str) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/jsonrpc", post(handle_rpc))
        .with_state(engine);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("JSON-RPC server listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}
