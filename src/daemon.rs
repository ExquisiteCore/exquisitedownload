use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::cli::RpcArgs;

/// Get the directory for PID and log files.
/// Windows: same directory as the executable
/// Other OS: ~/.config/edl/
fn data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            return dir.to_path_buf();
        }
        PathBuf::from(".")
    }

    #[cfg(not(target_os = "windows"))]
    {
        directories::ProjectDirs::from("", "", "edl")
            .map(|dirs| dirs.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn pid_path() -> PathBuf {
    data_dir().join("edl.pid")
}

pub fn log_path() -> PathBuf {
    data_dir().join("edl.log")
}

/// Write current process PID to file.
pub fn write_pid() -> Result<()> {
    let path = pid_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&path, std::process::id().to_string())
        .with_context(|| format!("Failed to write PID file: {}", path.display()))?;
    Ok(())
}

/// Remove PID file.
pub fn remove_pid() {
    let _ = fs::remove_file(pid_path());
}

/// Re-launch the current process in background without --daemon/-D flag.
pub fn daemonize() -> Result<()> {
    let exe = std::env::current_exe().context("Failed to get executable path")?;

    // Collect args, removing --daemon and -D
    let args: Vec<String> = std::env::args()
        .skip(1) // skip argv[0]
        .filter(|a| a != "--daemon" && a != "-D")
        .collect();

    let log_file_path = log_path();
    if let Some(parent) = log_file_path.parent() {
        fs::create_dir_all(parent).ok();
    }

    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .with_context(|| format!("Failed to open log file: {}", log_file_path.display()))?;

    let log_err = log_file
        .try_clone()
        .context("Failed to clone log file handle")?;

    let mut cmd = Command::new(&exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }

    let child = cmd.spawn().context("Failed to spawn daemon process")?;
    let pid = child.id();

    // Write child PID
    let pid_file = pid_path();
    if let Some(parent) = pid_file.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&pid_file, pid.to_string())?;

    println!("edl rpc 已在后台启动 (PID: {})", pid);
    println!("日志文件: {}", log_file_path.display());
    Ok(())
}

/// Stop the daemon process by reading PID file.
pub fn stop_daemon() -> Result<()> {
    let pid_file = pid_path();
    if !pid_file.exists() {
        bail!("PID 文件不存在，edl rpc 可能未在运行");
    }

    let pid_str = fs::read_to_string(&pid_file).context("Failed to read PID file")?;
    let pid = pid_str.trim();

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("taskkill")
            .args(["/PID", pid, "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("Failed to execute taskkill")?;
        if !status.success() {
            // Process might already be dead, clean up anyway
            println!("进程可能已退出");
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let status = Command::new("kill")
            .arg(pid)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("Failed to execute kill")?;
        if !status.success() {
            println!("进程可能已退出");
        }
    }

    let _ = fs::remove_file(&pid_file);
    println!("edl rpc 已停止 (PID: {})", pid);
    Ok(())
}

/// Register edl rpc as an auto-start service.
pub fn install_service(args: &RpcArgs) -> Result<()> {
    let exe = std::env::current_exe()
        .context("Failed to get executable path")?
        .display()
        .to_string();

    // Build the rpc command args
    // Windows uses -D flag so schtasks spawns a hidden process
    #[cfg(target_os = "windows")]
    let mut rpc_args = format!("\"{}\" rpc -D", exe);
    #[cfg(not(target_os = "windows"))]
    let mut rpc_args = format!("\"{}\" rpc", exe);
    if args.listen != "127.0.0.1:6800" {
        rpc_args.push_str(&format!(" --listen {}", args.listen));
    }
    if let Some(ref secret) = args.secret {
        rpc_args.push_str(&format!(" --secret {}", secret));
    }

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("schtasks")
            .args([
                "/create", "/tn", "edl-rpc", "/tr", &rpc_args, "/sc", "onlogon", "/rl", "highest",
                "/f",
            ])
            .status()
            .context("Failed to create scheduled task")?;
        if status.success() {
            println!("已注册开机自启 (Windows 计划任务: edl-rpc)");
            println!("命令: {}", rpc_args);
        } else {
            bail!("注册计划任务失败，请尝试以管理员身份运行");
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").context("HOME not set")?;
        let service_dir = PathBuf::from(&home).join(".config/systemd/user");
        fs::create_dir_all(&service_dir)?;

        let service_content = format!(
            "[Unit]\n\
             Description=ExquisiteDownload RPC Service\n\
             After=network.target\n\
             \n\
             [Service]\n\
             ExecStart={}\n\
             Restart=on-failure\n\
             RestartSec=5\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            rpc_args
        );

        let service_path = service_dir.join("edl.service");
        fs::write(&service_path, service_content)?;

        Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()
            .context("Failed to reload systemd")?;

        let status = Command::new("systemctl")
            .args(["--user", "enable", "edl"])
            .status()
            .context("Failed to enable service")?;

        if status.success() {
            println!("已注册开机自启 (systemd user service: edl)");
            println!("服务文件: {}", service_path.display());
            println!("启动服务: systemctl --user start edl");
        } else {
            bail!("注册 systemd 服务失败");
        }
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").context("HOME not set")?;
        let plist_dir = PathBuf::from(&home).join("Library/LaunchAgents");
        fs::create_dir_all(&plist_dir)?;

        // Split rpc_args into program and arguments for plist
        let plist_content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key><string>com.edl.rpc</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{exe}</string>\n\
             \t\t<string>rpc</string>\n\
             {listen_entry}\
             {secret_entry}\
             \t</array>\n\
             \t<key>RunAtLoad</key><true/>\n\
             \t<key>KeepAlive</key><true/>\n\
             </dict>\n\
             </plist>\n",
            exe = exe,
            listen_entry = if args.listen != "127.0.0.1:6800" {
                format!(
                    "\t\t<string>--listen</string>\n\t\t<string>{}</string>\n",
                    args.listen
                )
            } else {
                String::new()
            },
            secret_entry = if let Some(ref secret) = args.secret {
                format!(
                    "\t\t<string>--secret</string>\n\t\t<string>{}</string>\n",
                    secret
                )
            } else {
                String::new()
            },
        );

        let plist_path = plist_dir.join("com.edl.rpc.plist");
        fs::write(&plist_path, plist_content)?;

        let status = Command::new("launchctl")
            .args(["load", &plist_path.display().to_string()])
            .status()
            .context("Failed to load launch agent")?;

        if status.success() {
            println!("已注册开机自启 (launchd: com.edl.rpc)");
            println!("配置文件: {}", plist_path.display());
        } else {
            bail!("注册 launchd 服务失败");
        }
    }

    Ok(())
}

/// Unregister the auto-start service.
pub fn uninstall_service() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("schtasks")
            .args(["/delete", "/tn", "edl-rpc", "/f"])
            .status()
            .context("Failed to delete scheduled task")?;
        if status.success() {
            println!("已取消开机自启 (已删除计划任务: edl-rpc)");
        } else {
            bail!("删除计划任务失败，可能不存在或需要管理员权限");
        }
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("systemctl")
            .args(["--user", "disable", "edl"])
            .status()
            .ok();
        Command::new("systemctl")
            .args(["--user", "stop", "edl"])
            .status()
            .ok();

        let home = std::env::var("HOME").context("HOME not set")?;
        let service_path = PathBuf::from(&home).join(".config/systemd/user/edl.service");
        if service_path.exists() {
            fs::remove_file(&service_path)?;
        }

        Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()
            .ok();

        println!("已取消开机自启 (已删除 systemd 服务: edl)");
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").context("HOME not set")?;
        let plist_path = PathBuf::from(&home).join("Library/LaunchAgents/com.edl.rpc.plist");

        if plist_path.exists() {
            Command::new("launchctl")
                .args(["unload", &plist_path.display().to_string()])
                .status()
                .ok();
            fs::remove_file(&plist_path)?;
        }

        println!("已取消开机自启 (已删除 launchd 服务: com.edl.rpc)");
    }

    Ok(())
}
