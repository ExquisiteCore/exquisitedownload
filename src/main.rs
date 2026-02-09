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
    let mut config = config::load_config()?;

    match cli.command {
        Commands::Download(args) => {
            let speed_limit = args
                .limit_speed
                .as_deref()
                .map(config::parse_speed_limit)
                .transpose()?;

            let checksum = args
                .checksum
                .as_deref()
                .map(util::checksum::parse_checksum)
                .transpose()?;

            // CLI args override config file
            let proxy = args.proxy.or(config.proxy.take());
            let on_complete = args.on_complete.or(config.on_complete.take());

            let engine = DownloadEngine::with_proxy(config, proxy.as_deref())?;

            let task_id = engine
                .download(
                    args.url,
                    args.out,
                    args.dir.clone(),
                    args.split,
                    args.max_connections,
                    speed_limit,
                )
                .await?;

            // Checksum verification
            if let Some((algo, expected)) = checksum {
                let task = engine.get_task(&task_id).await;
                if let Some(task) = task {
                    info!("Verifying {} checksum...", algo);
                    util::checksum::verify_file(&task.file_path, algo, expected).await?;
                    info!("Checksum OK ({})", algo);
                }
            }

            // On-complete notification
            if let Some(ref cmd) = on_complete {
                let task = engine.get_task(&task_id).await;
                if let Some(task) = task {
                    let file_str = task.file_path.display().to_string();
                    let size = task.downloaded_bytes();
                    config::run_on_complete(cmd, &file_str, size, &task_id);
                }
            }

            info!("Task {} completed successfully", task_id);
        }

        Commands::Rpc(args) => {
            // CLI --secret overrides config file
            let secret = args.secret.or(config.rpc_secret.take());
            let engine = Arc::new(DownloadEngine::new(config)?);
            rpc::server::start_rpc_server(engine, &args.listen, secret).await?;
        }

        Commands::Status => {
            let resp = rpc_call("getGlobalStat", serde_json::json!({})).await?;
            let active_list = rpc_call("tellActive", serde_json::json!({})).await?;
            let waiting_list = rpc_call("tellWaiting", serde_json::json!({})).await?;
            let stopped_list = rpc_call("tellStopped", serde_json::json!({})).await?;

            println!("=== Global Stats ===");
            println!(
                "  Active: {}  |  Waiting: {}  |  Stopped: {}  |  Speed Limit: {}",
                resp.get("active").and_then(|v| v.as_u64()).unwrap_or(0),
                resp.get("waiting").and_then(|v| v.as_u64()).unwrap_or(0),
                resp.get("stopped").and_then(|v| v.as_u64()).unwrap_or(0),
                {
                    let limit = resp.get("speed_limit").and_then(|v| v.as_u64()).unwrap_or(0);
                    if limit == 0 { "none".to_string() } else { util::speed::format_bytes(limit) + "/s" }
                },
            );

            let all_tasks: Vec<&serde_json::Value> = [&active_list, &waiting_list, &stopped_list]
                .iter()
                .filter_map(|v| v.as_array())
                .flatten()
                .collect();

            if all_tasks.is_empty() {
                println!("\n  No tasks.");
            } else {
                println!(
                    "\n{:<38} {:<10} {:<20} {:<12} {}",
                    "GID", "STATUS", "FILE", "PROGRESS", "SIZE"
                );
                println!("{}", "-".repeat(95));

                for task in &all_tasks {
                    let gid = task.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = task.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    let file_path = task.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
                    let filename = std::path::Path::new(file_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(file_path);
                    let display_name = if filename.len() > 18 {
                        format!("{}...", &filename[..15])
                    } else {
                        filename.to_string()
                    };

                    let total_size = task.get("total_size").and_then(|v| v.as_u64()).unwrap_or(0);
                    let downloaded: u64 = task
                        .get("segments")
                        .and_then(|v| v.as_array())
                        .map(|segs| {
                            segs.iter()
                                .filter_map(|s| s.get("downloaded").and_then(|d| d.as_u64()))
                                .sum()
                        })
                        .unwrap_or(0);

                    let progress = if total_size > 0 {
                        format!("{:.1}%", downloaded as f64 / total_size as f64 * 100.0)
                    } else {
                        util::speed::format_bytes(downloaded)
                    };

                    let size_str = if total_size > 0 {
                        util::speed::format_bytes(total_size)
                    } else {
                        "unknown".to_string()
                    };

                    println!(
                        "{:<38} {:<10} {:<20} {:<12} {}",
                        gid, status, display_name, progress, size_str
                    );
                }
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
