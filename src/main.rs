mod cli;
mod config;
mod download_core;
mod net;
mod rpc;
mod storage;
mod util;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};
use config::Config;
use download_core::engine::DownloadEngine;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let config = Config::default();

    match cli.command {
        Commands::Download(args) => {
            let speed_limit = args
                .limit_speed
                .as_deref()
                .map(config::parse_speed_limit)
                .transpose()?;

            let engine = DownloadEngine::new(config)?;

            let task_id = engine
                .download(
                    args.url,
                    args.out,
                    args.dir,
                    args.split,
                    args.max_connections,
                    speed_limit,
                )
                .await?;

            info!("Task {} completed successfully", task_id);
        }

        Commands::Rpc(args) => {
            let engine = Arc::new(DownloadEngine::new(config)?);
            rpc::server::start_rpc_server(engine, &args.listen).await?;
        }

        Commands::Status => {
            let client = reqwest::Client::new();
            let resp: reqwest::Response = client
                .post("http://127.0.0.1:6800/jsonrpc")
                .json(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": "1",
                    "method": "getGlobalStat"
                }))
                .send()
                .await?;

            let body: serde_json::Value = resp.json().await?;
            if let Some(result) = body.get("result") {
                println!("Global Status:");
                println!(
                    "  Active: {}",
                    result.get("active").and_then(|v| v.as_u64()).unwrap_or(0)
                );
                println!(
                    "  Waiting: {}",
                    result.get("waiting").and_then(|v| v.as_u64()).unwrap_or(0)
                );
                println!(
                    "  Stopped: {}",
                    result.get("stopped").and_then(|v| v.as_u64()).unwrap_or(0)
                );
            } else if let Some(err) = body.get("error") {
                eprintln!("Error: {}", err);
            }
        }

        Commands::Pause { task_id } => {
            rpc_call("pause", serde_json::json!([task_id])).await?;
            println!("Task {} paused", task_id);
        }

        Commands::Resume { task_id } => {
            rpc_call("unpause", serde_json::json!([task_id])).await?;
            println!("Task {} resumed", task_id);
        }

        Commands::Remove { task_id } => {
            rpc_call("remove", serde_json::json!([task_id])).await?;
            println!("Task {} removed", task_id);
        }
    }

    Ok(())
}

async fn rpc_call(method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let resp: reqwest::Response = client
        .post("http://127.0.0.1:6800/jsonrpc")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": method,
            "params": params
        }))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    if let Some(error) = body.get("error") {
        anyhow::bail!("RPC error: {}", error);
    }
    Ok(body
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}
