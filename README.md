# ExquisiteDownload

高性能多连接 HTTP 下载管理器，Rust 实现，灵感来自 [Aria2](https://aria2.github.io/)。

## 特性

- **多连接分片下载** — 将文件拆分为多个分片并行下载，充分利用带宽
- **断点续传** — 下载状态自动持久化，中断后可从断点恢复
- **JSON-RPC 远程控制** — 兼容 Aria2 风格的 JSON-RPC 2.0 接口
- **实时进度显示** — 终端进度条，显示速度、已下载量和预计剩余时间
- **带宽限速** — 支持全局速度限制（`1M`、`500K` 等格式）
- **自动文件名识别** — 从 `Content-Disposition` 或 URL 路径自动提取文件名

## 安装

```bash
cargo install --path .
```

## 快速开始

### 下载文件

```bash
# 基础下载（默认 8 连接）
edl download https://example.com/file.zip

# 指定输出文件名和下载目录
edl download https://example.com/file.zip -o myfile.zip -d ~/downloads

# 16 连接分片下载
edl download https://example.com/file.zip -s 16

# 限速 1MB/s
edl download https://example.com/file.zip --limit-speed 1M
```

### JSON-RPC 服务器

```bash
# 启动 RPC 守护进程
edl rpc

# 指定监听地址
edl rpc --listen 0.0.0.0:6800
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
  <URL>                  下载链接

Options:
  -o, --out <FILE>       输出文件名
  -d, --dir <DIR>        下载目录（默认：系统下载文件夹）
  -s, --split <N>        分片数量 [默认: 8]
  -x, --max-connections  每任务最大连接数 [默认: 8]
      --limit-speed      限速（如 1M, 500K, 1024）
```

## JSON-RPC API

RPC 端点：`POST http://127.0.0.1:6800/jsonrpc`

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

# 查询全局状态
curl -X POST http://127.0.0.1:6800/jsonrpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"1","method":"getGlobalStat"}'

# 查询任务详情
curl -X POST http://127.0.0.1:6800/jsonrpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"1","method":"tellStatus","params":["<task_id>"]}'
```

## 项目结构

```
src/
├── main.rs                # 入口：CLI 解析与调度
├── cli.rs                 # clap 命令定义
├── config.rs              # 全局配置与默认值
├── download_core/
│   ├── engine.rs          # 下载引擎：任务调度与生命周期管理
│   ├── task.rs            # 下载任务数据结构
│   ├── segment.rs         # 文件分片逻辑
│   ├── worker.rs          # 分片下载协程
│   └── merge.rs           # 分片文件合并
├── net/
│   └── http.rs            # HTTP 客户端封装
├── storage/
│   └── state.rs           # 断点续传状态持久化
├── rpc/
│   └── server.rs          # JSON-RPC 服务器
└── util/
    ├── speed.rs           # 限速器与速度格式化
    └── progress.rs        # 进度条显示
```

## 技术栈

| 组件 | 库 |
|------|----|
| 异步运行时 | [tokio](https://tokio.rs) |
| HTTP 客户端 | [reqwest](https://docs.rs/reqwest) |
| CLI 框架 | [clap](https://docs.rs/clap) |
| RPC 服务器 | [axum](https://docs.rs/axum) |
| 序列化 | [serde](https://serde.rs) + serde_json |
| 进度条 | [indicatif](https://docs.rs/indicatif) |
| 日志 | [tracing](https://docs.rs/tracing) |
| 错误处理 | [anyhow](https://docs.rs/anyhow) + [thiserror](https://docs.rs/thiserror) |

## License

MIT
