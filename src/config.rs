use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub max_concurrent_tasks: usize,
    pub max_connections_per_task: u8,
    pub download_dir: PathBuf,
    pub max_speed: Option<u64>,
    pub rpc_listen_addr: SocketAddr,
    pub rpc_secret: Option<String>,
    pub user_agent: String,
    pub timeout_secs: u64,
    pub retry_count: u32,
    pub proxy: Option<String>,
    /// Command to run after download completes (supports {file}, {size}, {id})
    pub on_complete: Option<String>,
}

/// TOML-friendly config structure (all fields optional, uses strings)
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    max_concurrent_tasks: Option<usize>,
    max_connections_per_task: Option<u8>,
    download_dir: Option<String>,
    /// Speed limit as human string like "1M", "500K"
    max_speed: Option<String>,
    rpc_listen_addr: Option<String>,
    rpc_secret: Option<String>,
    user_agent: Option<String>,
    timeout_secs: Option<u64>,
    retry_count: Option<u32>,
    proxy: Option<String>,
    on_complete: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .and_then(|dirs| dirs.download_dir().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            max_concurrent_tasks: 5,
            max_connections_per_task: 8,
            download_dir,
            max_speed: None,
            rpc_listen_addr: "127.0.0.1:6800".parse().unwrap(),
            rpc_secret: None,
            user_agent: format!("ExquisiteDownload/{}", env!("CARGO_PKG_VERSION")),
            timeout_secs: 30,
            retry_count: 3,
            proxy: None,
            on_complete: None,
        }
    }
}

/// Get the config file path: ~/.config/edl/config.toml (or platform equivalent)
pub fn config_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "edl")
        .map(|dirs| dirs.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("edl.config.toml"))
}

/// Load config from file, falling back to defaults for missing fields.
/// Creates a default config file if none exists.
pub fn load_config() -> Result<Config> {
    let path = config_path();
    let mut config = Config::default();

    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        let file_cfg: FileConfig = toml::from_str(&content)
            .with_context(|| format!("failed to parse config file: {}", path.display()))?;

        // Merge file config into defaults
        if let Some(v) = file_cfg.max_concurrent_tasks {
            config.max_concurrent_tasks = v;
        }
        if let Some(v) = file_cfg.max_connections_per_task {
            config.max_connections_per_task = v;
        }
        if let Some(v) = file_cfg.download_dir {
            config.download_dir = PathBuf::from(v);
        }
        if let Some(v) = &file_cfg.max_speed {
            config.max_speed = Some(parse_speed_limit(v)?);
        }
        if let Some(v) = &file_cfg.rpc_listen_addr {
            config.rpc_listen_addr = v.parse()
                .with_context(|| format!("invalid rpc_listen_addr: {}", v))?;
        }
        if let Some(v) = file_cfg.rpc_secret {
            config.rpc_secret = Some(v);
        }
        if let Some(v) = file_cfg.user_agent {
            config.user_agent = v;
        }
        if let Some(v) = file_cfg.timeout_secs {
            config.timeout_secs = v;
        }
        if let Some(v) = file_cfg.retry_count {
            config.retry_count = v;
        }
        if let Some(v) = file_cfg.proxy {
            config.proxy = Some(v);
        }
        if let Some(v) = file_cfg.on_complete {
            config.on_complete = Some(v);
        }

        info!("Loaded config from {}", path.display());
    } else {
        // Create default config file for the user
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let default_content = generate_default_config();
        if std::fs::write(&path, &default_content).is_ok() {
            info!("Created default config at {}", path.display());
        }
    }

    Ok(config)
}

fn generate_default_config() -> String {
    let download_dir = directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(|d| d.display().to_string()))
        .unwrap_or_else(|| ".".to_string());

    format!(
        r#"# ExquisiteDownload (edl) Configuration
# Lines starting with # are comments. Uncomment to override defaults.

# Maximum concurrent download tasks
# max_concurrent_tasks = 5

# Maximum connections per task
# max_connections_per_task = 8

# Default download directory
# download_dir = "{download_dir}"

# Global speed limit (e.g., "1M", "500K", "0" for unlimited)
# max_speed = "0"

# RPC server listen address
# rpc_listen_addr = "127.0.0.1:6800"

# RPC authentication secret (leave empty to disable)
# rpc_secret = "your_secret_here"

# HTTP User-Agent
# user_agent = "ExquisiteDownload/{version}"

# Connection timeout in seconds
# timeout_secs = 30

# Retry count on failure
# retry_count = 3

# HTTP/HTTPS/SOCKS5 proxy
# proxy = "http://127.0.0.1:7890"

# Command to execute after download completes
# Supports placeholders: {{file}} {{size}} {{id}}
# on_complete = "notify-send 'Download complete' '{{file}}'"
"#,
        download_dir = download_dir.replace('\\', "\\\\"),
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// Parse speed limit string like "1M", "500K", "1024" into bytes/sec
pub fn parse_speed_limit(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() || s == "0" {
        return Ok(0);
    }

    let (num_str, multiplier) = if let Some(n) = s.strip_suffix(['k', 'K']) {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix(['m', 'M']) {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix(['g', 'G']) {
        (n, 1024 * 1024 * 1024)
    } else {
        (s, 1u64)
    };

    let num: u64 = num_str.parse()?;
    Ok(num * multiplier)
}

/// Execute on-complete command with placeholder substitution
pub fn run_on_complete(cmd_template: &str, file_path: &str, size: u64, task_id: &str) {
    let cmd = cmd_template
        .replace("{file}", file_path)
        .replace("{size}", &size.to_string())
        .replace("{id}", task_id);

    info!("Running on-complete: {}", cmd);

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd").args(["/C", &cmd]).spawn();

    #[cfg(not(target_os = "windows"))]
    let result = std::process::Command::new("sh").args(["-c", &cmd]).spawn();

    match result {
        Ok(_) => {}
        Err(e) => tracing::warn!("on-complete command failed: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_speed_limit() {
        assert_eq!(parse_speed_limit("1024").unwrap(), 1024);
        assert_eq!(parse_speed_limit("1K").unwrap(), 1024);
        assert_eq!(parse_speed_limit("1k").unwrap(), 1024);
        assert_eq!(parse_speed_limit("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_speed_limit("2G").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_speed_limit("0").unwrap(), 0);
    }
}
