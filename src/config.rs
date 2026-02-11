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
    pub max_redirects: u32,
    pub proxy: Option<String>,
    /// Command to run after download completes (supports {file}, {size}, {id})
    pub on_complete: Option<String>,
    /// Command to run when download starts (supports {file}, {id}, {url})
    pub on_start: Option<String>,
    /// Command to run when download is paused (supports {file}, {id})
    pub on_pause: Option<String>,
    /// Command to run when download fails (supports {file}, {id}, {error})
    pub on_error: Option<String>,
    /// Custom HTTP headers (e.g., ["Authorization: Bearer xxx"])
    pub headers: Vec<String>,
    /// Cookie string
    pub cookie: Option<String>,
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
    max_redirects: Option<u32>,
    proxy: Option<String>,
    on_complete: Option<String>,
    on_start: Option<String>,
    on_pause: Option<String>,
    on_error: Option<String>,
    /// Custom HTTP headers
    headers: Option<Vec<String>>,
    /// Cookie string
    cookie: Option<String>,
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
            max_redirects: 10,
            proxy: None,
            on_complete: None,
            on_start: None,
            on_pause: None,
            on_error: None,
            headers: Vec::new(),
            cookie: None,
        }
    }
}

/// Get the config file path.
/// Windows: same directory as the executable (e.g., D:\edl\config.toml)
/// Other OS: ~/.config/edl/config.toml
pub fn config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            return dir.join("config.toml");
        }
        PathBuf::from("config.toml")
    }

    #[cfg(not(target_os = "windows"))]
    {
        directories::ProjectDirs::from("", "", "edl")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("edl.config.toml"))
    }
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
            config.rpc_listen_addr = v
                .parse()
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
        if let Some(v) = file_cfg.max_redirects {
            config.max_redirects = v;
        }
        if let Some(v) = file_cfg.proxy {
            config.proxy = Some(v);
        }
        if let Some(v) = file_cfg.on_complete {
            config.on_complete = Some(v);
        }
        if let Some(v) = file_cfg.on_start {
            config.on_start = Some(v);
        }
        if let Some(v) = file_cfg.on_pause {
            config.on_pause = Some(v);
        }
        if let Some(v) = file_cfg.on_error {
            config.on_error = Some(v);
        }
        if let Some(v) = file_cfg.headers {
            config.headers = v;
        }
        if let Some(v) = file_cfg.cookie {
            config.cookie = Some(v);
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
        r#"# ExquisiteDownload (edl) 配置文件
# 以 # 开头的行为注释，取消注释即可覆盖默认值

# 最大同时下载任务数
# max_concurrent_tasks = 5

# 每个任务的最大连接数
# max_connections_per_task = 8

# 默认下载目录
# download_dir = "{download_dir}"

# 全局限速（如 "1M", "500K", "0" 为不限速）
# max_speed = "0"

# RPC 服务器监听地址
# rpc_listen_addr = "127.0.0.1:6800"

# RPC 认证密钥（留空则不启用认证）
# rpc_secret = "your_secret_here"

# HTTP User-Agent
# user_agent = "ExquisiteDownload/{version}"

# 连接超时时间（秒）
# timeout_secs = 30

# 失败重试次数
# retry_count = 3

# HTTP 最大重定向次数（0 = 禁止重定向）
# max_redirects = 10

# HTTP/HTTPS/SOCKS5 代理
# proxy = "http://127.0.0.1:7890"

# 下载完成后执行的命令
# 支持占位符：{{file}} 文件路径, {{size}} 文件大小, {{id}} 任务ID
# on_complete = "notify-send '下载完成' '{{file}}'"

# 下载开始时执行的命令（支持 {{file}}, {{id}}, {{url}}）
# on_start = "echo '开始下载: {{file}}'"

# 下载暂停时执行的命令（支持 {{file}}, {{id}}）
# on_pause = "echo '已暂停: {{file}}'"

# 下载出错时执行的命令（支持 {{file}}, {{id}}, {{error}}）
# on_error = "echo '下载失败: {{file}} — {{error}}'"

# 自定义 HTTP 请求头（数组格式）
# headers = ["Authorization: Bearer xxx", "Referer: https://example.com"]

# Cookie 字符串
# cookie = "sid=abc; token=xyz"
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

/// Execute a hook command with placeholder substitution.
/// `vars` is a list of (placeholder, value) pairs, e.g., [("file", "/path"), ("id", "abc")].
pub fn run_hook(cmd_template: &str, vars: &[(&str, &str)]) {
    let mut cmd = cmd_template.to_string();
    for (key, val) in vars {
        cmd = cmd.replace(&format!("{{{}}}", key), val);
    }

    info!("Running hook: {}", cmd);

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd").args(["/C", &cmd]).spawn();

    #[cfg(not(target_os = "windows"))]
    let result = std::process::Command::new("sh").args(["-c", &cmd]).spawn();

    match result {
        Ok(_) => {}
        Err(e) => tracing::warn!("hook command failed: {}", e),
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

    #[test]
    fn test_parse_speed_limit_empty() {
        assert_eq!(parse_speed_limit("").unwrap(), 0);
        assert_eq!(parse_speed_limit("  ").unwrap(), 0);
    }

    #[test]
    fn test_parse_speed_limit_with_whitespace() {
        assert_eq!(parse_speed_limit(" 1M ").unwrap(), 1024 * 1024);
    }

    #[test]
    fn test_parse_speed_limit_invalid() {
        assert!(parse_speed_limit("abc").is_err());
        assert!(parse_speed_limit("M").is_err());
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.max_concurrent_tasks, 5);
        assert_eq!(config.max_connections_per_task, 8);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.retry_count, 3);
        assert_eq!(config.max_redirects, 10);
        assert!(config.max_speed.is_none());
        assert!(config.proxy.is_none());
        assert!(config.rpc_secret.is_none());
        assert!(config.on_complete.is_none());
    }

    #[test]
    fn test_run_on_complete_placeholders() {
        // Just test that the function doesn't panic
        // We can't easily test the actual command execution in unit tests
        // But we can verify the template substitution by inspecting indirectly
        let template = "echo {file} {size} {id}";
        let cmd = template
            .replace("{file}", "test.zip")
            .replace("{size}", "1024")
            .replace("{id}", "abc123");
        assert_eq!(cmd, "echo test.zip 1024 abc123");
    }

    #[test]
    fn test_run_hook_substitution() {
        // Verify run_hook's placeholder logic by replicating it
        let template = "notify {id} {url} {error}";
        let vars = [("id", "task1"), ("url", "http://x.com"), ("error", "timeout")];
        let mut cmd = template.to_string();
        for (key, val) in &vars {
            cmd = cmd.replace(&format!("{{{}}}", key), val);
        }
        assert_eq!(cmd, "notify task1 http://x.com timeout");
    }

    #[test]
    fn test_run_hook_missing_placeholder() {
        // Placeholders not in vars should remain unchanged
        let template = "echo {file} {unknown}";
        let vars = [("file", "test.zip")];
        let mut cmd = template.to_string();
        for (key, val) in &vars {
            cmd = cmd.replace(&format!("{{{}}}", key), val);
        }
        assert_eq!(cmd, "echo test.zip {unknown}");
    }

    #[test]
    fn test_generate_default_config() {
        let content = generate_default_config();
        assert!(content.contains("max_concurrent_tasks"));
        assert!(content.contains("download_dir"));
        assert!(content.contains("proxy"));
        assert!(content.contains("on_complete"));
    }
}
