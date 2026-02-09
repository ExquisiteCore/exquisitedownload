use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "edl", about = "ExquisiteDownload - A high-performance download manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Download a file from URL
    #[command(alias = "d")]
    Download(DownloadArgs),

    /// Start JSON-RPC server daemon
    Rpc(RpcArgs),

    /// Show download status (connects to RPC server)
    Status,

    /// Pause a download task
    Pause {
        /// Task ID to pause
        task_id: String,
    },

    /// Resume a paused download task
    Resume {
        /// Task ID to resume
        task_id: String,
    },

    /// Remove a download task
    Remove {
        /// Task ID to remove
        task_id: String,
    },
}

#[derive(clap::Args)]
pub struct DownloadArgs {
    /// URL to download
    pub url: String,

    /// Output file name
    #[arg(short, long)]
    pub out: Option<String>,

    /// Download directory
    #[arg(short, long)]
    pub dir: Option<PathBuf>,

    /// Number of segments (default: 8)
    #[arg(short, long, default_value_t = 8)]
    pub split: u8,

    /// Max connections per task
    #[arg(short = 'x', long, default_value_t = 8)]
    pub max_connections: u8,

    /// Speed limit (e.g., 1M, 500K)
    #[arg(long)]
    pub limit_speed: Option<String>,

    /// Verify checksum after download (e.g., sha256=abcdef..., md5=..., sha1=...)
    #[arg(long)]
    pub checksum: Option<String>,

    /// HTTP/HTTPS/SOCKS5 proxy (e.g., http://127.0.0.1:7890, socks5://...)
    #[arg(long)]
    pub proxy: Option<String>,

    /// Command to run after download completes (supports {file}, {size}, {id})
    #[arg(long)]
    pub on_complete: Option<String>,
}

#[derive(clap::Args)]
pub struct RpcArgs {
    /// Listen address
    #[arg(long, default_value = "127.0.0.1:6800")]
    pub listen: String,

    /// RPC secret token
    #[arg(long)]
    pub secret: Option<String>,
}
