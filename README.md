# Pageville

Pageville 是一个本地优先的静态页面托管器：一个 Rust 二进制同时提供
CLI 和 loopback daemon，把页面发布成不可变快照，并为页面与 Agent 提供一套
带版本上下文的事件 API。

> v0 状态：功能闭环已具备，适合本地开发、设计预览和受控的 HITL 工具页。
> v0 默认只监听本机回环地址，尚未承诺远程部署、鉴权、自动清理或公网托管。

## 能力概览

- **一条命令发布**：`publish` 在 daemon 未运行时自动启动服务。
- **不可变快照**：文件按 BLAKE3 内容寻址，重复内容跨快照去重；每次发布得到
  稳定的 `snapshot_id`。
- **稳定 URL**：`/{page}/` 和 `/{page}/latest/` 指向最新快照，
  `/{page}/{snapshot_id}/` 永久钉住历史版本。
- **Atlas 项目大厅**：打开 `/` 可浏览已发布页面（以项目呈现）、查看最新快照、
  历史版本与事件数量，并直接打开或复制稳定 URL；大厅本身随单二进制内嵌，
  不依赖前端构建或外部字体。
- **SPA 与 MIME**：发布时可启用 `--spa`；未命中文件回落到同一快照的
  `index.html`，其他请求返回 404；响应按扩展名推断 MIME。
- **HITL 事件闭环**：页面追加写入统一事件 envelope，Agent 可按会话、时间和
  快照版本过滤读取 NDJSON。
- **协议先行**：CLI 的所有数据操作都经 `/api/v0/` HTTP/JSON 完成，
  `--target` 为后续 self-host/远程端点保留兼容面（v0 仍只接受 loopback URL）。

## 快速开始

需要 Rust stable、`curl`；运行验收脚本还需要 `jq`。

也可以把二进制安装到 Cargo 的本地 bin 目录：

```bash
cargo install --path .
pageville --version
```

```bash
# 构建
cargo build --release

# 准备一个页面目录
mkdir -p site
printf '<h1>Hello Pageville</h1>\n' > site/index.html

# 发布；首次调用会自动启动 daemon
./target/release/pageville publish site --page demo

# 访问最新快照
curl -i http://127.0.0.1:7777/demo/

# 打开项目大厅
open http://127.0.0.1:7777/

# 完成后停止本地 daemon
./target/release/pageville daemon stop
```

默认端口是 `7777`。可以在启动 daemon 和调用 CLI 时同时设置环境变量，
把运行数据放到一个明确的数据目录：

```bash
run_dir="$(mktemp -d)"
export PAGEVILLE_DATA_DIR="$run_dir/pageville-data"
export PAGEVILLE_PORT=17777

./target/release/pageville publish site --page demo
curl http://127.0.0.1:17777/demo/
./target/release/pageville daemon stop
```

## CLI

```text
pageville publish <dir> --page <slug> [--spa]
pageville pages list
pageville versions list --page <slug>
pageville events pull --page <slug>
  [--session <id>] [--since <RFC3339>] [--until <RFC3339>]
  [--version <snapshot-id>]
pageville daemon status|start|stop
pageville --target <loopback-url> ...
```

全局 `--json` 适用于列表和 daemon 状态输出；`events pull` 默认每行输出一个
JSON 对象（NDJSON）。例如：

```bash
pageville --json pages list
pageville --json versions list --page demo
pageville events pull --page demo --session review-1
```

页面 slug 只允许小写字母、数字和连字符，长度不超过 128，且不能使用
`api` 或 `latest`。

## URL 与版本模型

| URL | 含义 |
| --- | --- |
| `/` | Pageville Atlas 项目大厅（读取 `/api/v0/` 数据面） |
| `/{page}/` | 当前 `latest` 快照的 `index.html` |
| `/{page}/latest/<path>` | 当前 `latest` 快照中的指定文件 |
| `/{page}/{snapshot_id}/<path>` | 指定不可变快照中的文件 |

发布完成后，服务端先写入全部对象和 manifest，再在 SQLite 事务中登记快照并
切换 `latest`。因此历史 URL 不会随新发布改变，最新 URL 只会在完整快照可用后
切换。

## HTTP API

API 前缀固定为 `/api/v0/`。CLI 是这套协议的第一个客户端。

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| `POST` | `/api/v0/pages/{page}/snapshots` | 发布快照 |
| `GET` | `/api/v0/pages` | 列出页面及其 latest |
| `GET` | `/api/v0/pages/{page}/snapshots` | 列出历史快照 |
| `POST` | `/api/v0/pages/{page}/events` | 追加事件 |
| `GET` | `/api/v0/pages/{page}/events` | 过滤读取事件（NDJSON） |
| `GET` | `/api/v0/context?path=...` | 根据页面 URL 查询真实快照版本 |
| `GET` | `/api/v0/health` | 健康检查和协议版本 |
| `POST` | `/api/v0/shutdown` | 请求 daemon 优雅退出 |

### 发布快照

v0 的请求体是 JSON；`files` 的键为相对路径，值为标准 Base64 文件内容：

```bash
curl -sS -X POST \
  http://127.0.0.1:7777/api/v0/pages/demo/snapshots \
  -H 'content-type: application/json' \
  -d '{"files":{"index.html":"PGgxPkhlbGxvPC9oMT4K"},"spa":false}'
```

响应包含 `page`、`snapshot_id`、`url` 和 `spa`。同一页面的相同文件内容与相同
SPA 标志会得到相同的快照 ID；页面名也参与快照寻址，从而避免相同内容在不同
页面之间产生快照主键冲突。

### 追加与读取事件

写入方只提供 `version`（必须是真实快照 ID）、`session` 和任意 JSON `payload`；
`event_id`、`ts` 和 `identity` 由宿主生成：

```bash
snapshot_id="$(pageville --json versions list --page demo | jq -r '.[0].snapshot_id')"
curl -sS -X POST \
  http://127.0.0.1:7777/api/v0/pages/demo/events \
  -H 'content-type: application/json' \
  -d "{\"version\":\"$snapshot_id\",\"session\":\"review-1\",\"payload\":{\"rating\":5}}"

curl -sS \
  "http://127.0.0.1:7777/api/v0/pages/demo/events?session=review-1&version=$snapshot_id"
```

页面经 `/{page}/` 或 `/{page}/latest/` 访问时，可读取响应头
`X-Pageville-Page`、`X-Pageville-Version`；无法读取响应头的页面可调用
`/api/v0/context?path=/demo/` 获取真实版本。事件 API 拒绝把 `latest` 当作
持久化版本值。

## 数据目录与安全边界

默认数据目录为 `~/.pageville`，也可由 `PAGEVILLE_DATA_DIR` 指定。目录结构为：

```text
<data-dir>/
├── objects/       # BLAKE3 内容对象
├── manifests/     # 快照 manifest
├── pageville.db   # SQLite 元数据与事件（WAL）
├── daemon.pid
└── daemon.lock
```

daemon 固定 bind `127.0.0.1`，v0 不提供鉴权；不要把端口转发到局域网或公网。
快照内的文件路径必须是相对路径，服务端会拒绝绝对路径和目录穿越片段。事件 payload
不做业务语义校验，也不会自动脱敏，调用方应避免写入秘密或个人敏感信息。

## 验证与开发

```bash
cargo fmt -- --check
cargo check --all-targets
cargo test --all-targets
bash scripts/verify/t008-acceptance.sh
```

`t008-acceptance.sh` 会在各自的临时数据目录中串行执行七个验收切片，覆盖发布、
CAS 幂等、版本路由、SPA fallback、事件 envelope、CLI 面和 daemon 生命周期。
单项脚本可用于定位失败：`t001` 到 `t007` 的名称对应上述能力顺序。

## 当前限制与后续方向

以下边界是 v0 的明确取舍，而不是隐藏承诺：

- 没有远程/self-host 端点、用户鉴权、局域网暴露或公网 CDN。
- 没有快照 GC、保留策略、订阅/回调和内建 HITL 事件类型。
- daemon 生命周期、存储迁移和协议兼容性仍以本地单机使用为目标；升级前应
  备份数据目录。daemon lock 采用进程级文件锁，进程崩溃后锁会由操作系统释放，
  后续启动不会被遗留文件名卡住。
- 当前验证以黑盒 shell 验收为主，后续可补充更细的 Rust 单元/集成测试和发布
  自动化。

详细的范围、架构、数据模型与验收标准见 [`docs/PRD/`](docs/PRD/)。

## 贡献与发布说明

提交变更前请运行格式化、编译和完整验收。提交信息应描述一个清晰的意图边界，
不要把本地运行日志、构建产物、`.tmp`、`.codex` 或 `.bagakit` 运行时文件加入
产品提交。许可证和公开发布流程尚未在 v0 中声明；在对外分发前应由项目维护者
补充相应政策。
