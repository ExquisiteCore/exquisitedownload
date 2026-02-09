use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Maximum concurrent download tasks
    pub max_concurrent_tasks: usize,
    /// Maximum connections per task
    pub max_connections_per_task: u8,
    /// Default download directory
    pub download_dir: PathBuf,
    /// Global speed limit in bytes/sec (None = unlimited)
    pub max_speed: Option<u64>,
    /// RPC listen address
    pub rpc_listen_addr: SocketAddr,
    /// RPC authentication secret
    pub rpc_secret: Option<String>,
    /// User-Agent header
    pub user_agent: String,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
    /// Retry count on failure
    pub retry_count: u32,
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
        }
    }
}

/// Parse speed limit string like "1M", "500K", "1024" into bytes/sec
pub fn parse_speed_limit(s: &str) -> anyhow::Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty speed limit");
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
    }
}
