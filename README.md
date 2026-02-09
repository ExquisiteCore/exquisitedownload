# ExquisiteDownload

高性能多连接 HTTP 下载管理器，Rust 实现，灵感来自 [Aria2](https://aria2.github.io/)。

## 特性

- **多连接分片下载** — 将文件拆分为多个分片并行下载，充分利用带宽
- **断点续传** — 下载状态自动持久化，中断后可从断点恢复
- **JSON-RPC 远程控制** — 兼容 Aria2 风格的 JSON-RPC 2.0 接口，支持 Token 认证
- **实时进度显示** — 终端进度条，显示速度、已下载量和预计剩余时间
- **带宽限速** — 支持全局速度限制（`1M`、`500K` 等格式）
- **自动文件名识别** — 从 `Content-Disposition` 或 URL 路径自动提取文件名
- **校验和验证** — 下载后自动校验 SHA-256 / SHA-1 / MD5
- **代理支持** — HTTP / HTTPS / SOCKS5 代理
- **自定义 Header / Cookie** — 支持需要认证的下载场景
- **下载完成通知** — 下载完成后执行自定义命令
- **配置文件** — TOML 格式持久化配置，CLI 参数可覆盖
- **下载队列** — 并发任务数限制，自动排队
- **Chrome 浏览器扩展** — 拦截浏览器下载、右键菜单一键发送到 edl、弹窗任务管理

## 安装

```bash
cargo install --path .
```

或从 [GitHub Releases](https://github.com/nicexiaolong/exquisitedownload/releases) 下载预编译二进制文件。

## 快速开始

### 下载文件

```bash
# 基础下载（默认 8 连接）
edl download https://example.com/file.zip

# 指定输出文件名和下载目录
edl download https://example.com/file.zip -o myfile.zip -d ~/downloads

# 16 连接分片下载，限速 1MB/s
edl download https://example.com/file.zip -s 16 --limit-speed 1M

# 通过代理下载
edl download https://example.com/file.zip --proxy socks5://127.0.0.1:1080

# 自定义 Header 和 Cookie（用于需要认证的下载）
edl download https://example.com/file.zip --header "Authorization: Bearer token" --cookie "sid=abc"

# 下载后校验 SHA-256
edl download https://example.com/file.zip --checksum sha256=e3b0c44298fc...

# 下载完成后执行命令
edl download https://example.com/file.zip --on-complete "echo 完成: {file}"
```

### JSON-RPC 服务器

```bash
# 启动 RPC 守护进程
edl rpc

# 指定监听地址和认证密钥
edl rpc --listen 0.0.0.0:6800 --secret mysecret
```

### 任务管理（通过 RPC）

```bash
edl status              # 查看全局状态
edl pause <TASK_ID>     # 暂停任务
edl resume <TASK_ID>    # 恢复任务
edl remove <TASK_ID>    # 删除任务
```

## CLI 参数

```
edl download [OPTIONS] <URL>

Arguments:
  <URL>                      下载链接

Options:
  -o, --out <FILE>           输出文件名
  -d, --dir <DIR>            下载目录（默认：系统下载文件夹）
  -s, --split <N>            分片数量 [默认: 8]
  -x, --max-connections <N>  每任务最大连接数 [默认: 8]
      --limit-speed <SPEED>  限速（如 1M, 500K, 1024）
      --proxy <URL>          代理（http/https/socks5）
      --header <HEADER>      自定义 HTTP 头（可多次使用）
      --cookie <COOKIE>      Cookie 字符串
      --checksum <ALGO=HEX>  校验和（sha256/sha1/md5）
      --on-complete <CMD>    下载完成后执行命令（支持 {file} {size} {id}）
```

```
edl rpc [OPTIONS]

Options:
      --listen <ADDR>        监听地址 [默认: 127.0.0.1:6800]
      --secret <TOKEN>       RPC 认证密钥
```

## 配置文件

首次运行自动生成配置文件：

| 平台 | 路径 |
|------|------|
| Windows | edl.exe 同目录下 `config.toml` |
| Linux / macOS | `~/.config/edl/config.toml` |

```toml
# 最大同时下载任务数
max_concurrent_tasks = 5

# 每个任务的最大连接数
max_connections_per_task = 8

# 默认下载目录
download_dir = "C:\\Users\\you\\Downloads"

# 全局限速（"1M" = 1MB/s, "500K" = 500KB/s, "0" = 不限速）
max_speed = "0"

# RPC 服务器监听地址
rpc_listen_addr = "127.0.0.1:6800"

# RPC 认证密钥
rpc_secret = "your_secret_here"

# HTTP User-Agent
user_agent = "ExquisiteDownload/0.1.0"

# 连接超时（秒）
timeout_secs = 30

# 失败重试次数
retry_count = 3

# 代理
proxy = "http://127.0.0.1:7890"

# 下载完成后执行命令
on_complete = "notify-send '下载完成' '{file}'"

# 自定义 HTTP 请求头
headers = ["Authorization: Bearer xxx", "Referer: https://example.com"]

# Cookie 字符串
cookie = "sid=abc; token=xyz"
```

CLI 参数优先级高于配置文件。

## JSON-RPC API

RPC 端点：`POST http://127.0.0.1:6800/jsonrpc`

设置了 `--secret` 时，需在 params 首位传入 `"token:SECRET"`（Aria2 风格）。

### 方法列表

| 方法 | 参数 | 说明 |
|------|------|------|
| `addUri` | `[url]` 或 `{url, out, split}` | 添加下载任务 |
| `pause` | `[task_id]` | 暂停任务 |
| `unpause` | `[task_id]` | 恢复任务 |
| `remove` | `[task_id]` | 删除任务 |
| `tellStatus` | `[task_id]` | 查询任务状态 |
| `tellActive` | — | 查询所有活动任务 |
| `tellWaiting` | — | 查询等待中的任务 |
| `tellStopped` | — | 查询已完成/出错的任务 |
| `getGlobalStat` | — | 全局统计信息 |
| `changeGlobalOption` | `{max-overall-download-limit}` | 修改全局配置 |

### 调用示例

```bash
# 添加下载任务
curl -X POST http://127.0.0.1:6800/jsonrpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"1","method":"addUri","params":["https://example.com/file.zip"]}'

# 带认证的调用
curl -X POST http://127.0.0.1:6800/jsonrpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"1","method":"addUri","params":["token:mysecret","https://example.com/file.zip"]}'

# 查询全局状态
curl -X POST http://127.0.0.1:6800/jsonrpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"1","method":"getGlobalStat"}'

# 查询任务详情
curl -X POST http://127.0.0.1:6800/jsonrpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"1","method":"tellStatus","params":["<task_id>"]}'
```

## Chrome 浏览器扩展

配套 Chrome 扩展可以拦截浏览器下载并转发给 edl，也可以通过右键菜单和弹窗手动管理任务。

### 安装扩展

**方式一：从 Release 下载**

1. 前往 [GitHub Releases](https://github.com/nicexiaolong/exquisitedownload/releases) 下载 `edl-chrome-extension.zip`
2. 解压到任意目录
3. 打开 Chrome，进入 `chrome://extensions/`
4. 开启右上角**开发者模式**
5. 点击**加载已解压的扩展程序**，选择解压后的文件夹

**方式二：从源码加载**

1. 克隆项目仓库
2. 打开 `chrome://extensions/`，开启开发者模式
3. 点击加载已解压的扩展程序，选择项目下的 `extension/` 目录

### 使用前提

扩展需要 edl RPC 服务正在运行：

```bash
edl rpc
# 或指定密钥
edl rpc --secret mysecret
```

### 功能说明

**自动拦截下载**

开启后，浏览器中触发的下载会自动被 edl 接管（可在设置中配置文件大小阈值和文件类型过滤）。如果 edl 未运行，会自动回退到浏览器原生下载。

**右键菜单**

在任意链接上右键，选择「使用 edl 下载」即可将链接发送给 edl。

**弹窗任务管理**

点击工具栏图标打开弹窗：
- 顶部显示 edl 连接状态和全局统计（活跃/等待/完成）
- 中间为任务列表，可暂停、恢复、删除任务
- 底部输入框可手动粘贴 URL 添加下载

**扩展设置**

点击弹窗右上角齿轮图标打开设置页，可配置：

| 配置项 | 说明 | 默认值 |
|--------|------|--------|
| RPC 地址 | edl RPC 服务地址 | `http://127.0.0.1:6800/jsonrpc` |
| RPC 密钥 | 对应 `edl rpc --secret` 的密钥 | 空（不认证） |
| 自动拦截 | 是否自动拦截浏览器下载 | 开启 |
| 最小文件大小 | 小于此大小的文件不拦截 | 1 MB |
| 拦截文件类型 | 逗号分隔的扩展名 | `.zip,.exe,.mp4` 等常见格式 |

设置页提供「测试连接」按钮，可验证 RPC 地址和密钥是否正确。

## 项目结构

```
src/
├── main.rs                # 入口：CLI 解析与调度
├── cli.rs                 # clap 命令定义
├── config.rs              # 全局配置（TOML 持久化）
├── download_core/
│   ├── engine.rs          # 下载引擎：任务调度与生命周期管理
│   ├── task.rs            # 下载任务数据结构
│   ├── segment.rs         # 文件分片逻辑
│   ├── worker.rs          # 分片下载协程
│   └── merge.rs           # 分片文件合并
├── net/
│   └── http.rs            # HTTP 客户端封装（Header/Cookie/代理）
├── storage/
│   └── state.rs           # 断点续传状态持久化
├── rpc/
│   └── server.rs          # JSON-RPC 服务器（Token 认证 + CORS）
└── util/
    ├── speed.rs           # 限速器与速度格式化
    ├── progress.rs        # 进度条显示
    └── checksum.rs        # 校验和验证（SHA-256/SHA-1/MD5）

extension/                   # Chrome 浏览器扩展
├── manifest.json            # Manifest V3 配置
├── background.js            # Service Worker：下载拦截、右键菜单、RPC 通信
├── popup.html / js / css    # 弹窗界面：任务列表与手动添加
├── options.html / js        # 设置页面
└── icons/                   # 扩展图标（16/48/128px）
```

## 技术栈

| 组件 | 库 |
|------|----|
| 异步运行时 | [tokio](https://tokio.rs) |
| HTTP 客户端 | [reqwest](https://docs.rs/reqwest) (rustls) |
| CLI 框架 | [clap](https://docs.rs/clap) |
| RPC 服务器 | [axum](https://docs.rs/axum) |
| 序列化 | [serde](https://serde.rs) + serde_json + toml |
| 进度条 | [indicatif](https://docs.rs/indicatif) |
| 日志 | [tracing](https://docs.rs/tracing) |
| 校验 | [sha2](https://docs.rs/sha2) / [sha1](https://docs.rs/sha1) / [md-5](https://docs.rs/md-5) |
| 错误处理 | [anyhow](https://docs.rs/anyhow) + [thiserror](https://docs.rs/thiserror) |

## License

MIT
