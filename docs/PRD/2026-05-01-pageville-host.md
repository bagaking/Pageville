# Pageville v0 PRD：本地页面托管 + HITL 数据面 + Agent CLI 闭环

- 状态：已定稿（brainstorm 全部裁决已锁定，专家论坛 approved）
- 决策来源：需求讨论与专家评审记录（Q-001=B、Q-002=A+默认latest、Q-003=B、Q-004/Q-005 defer、架构 O1 primary）
- 执行跟踪：验证脚本与本文档「验收标准」；运行时跟踪文件不属于产品发布物。

---

## 1. 背景 / 问题

本地开发与 Agent 协作过程中会持续产生大量页面：设计预览、feature-tracker 状态页、HITL 交互页、一次性报告页等。现状是**每页一进程**的 ad-hoc 托管。各种 `vite dev`、`python -m http.server`、skill 自起 server 并存，带来：

- 端口冲突与进程泄漏，资源浪费；
- URL 无语义、不可收藏、不可交给别人（包括交给 Agent）；
- 页面没有版本概念，覆盖即丢失，历史不可回看；
- HITL 页面收集的人类输入（表单、评分、批注）无统一归宿，Agent 取数各写各的。

Pageville 用**一个本地常驻服务**收敛以上全部问题：统一托管、语义化 URL、内建不可变版本化，并把「页面写数据 → Agent 取数据」做成宿主一等能力。

## 2. 目标 / 非目标

### 目标（v0）

1. **一条命令发布**：把一个静态页面目录挂到语义化 URL 上并立即可访问。
2. **不可变版本化**：每次发布产生不可变快照；历史版本永远可回看；`latest` 指向最新且切换原子。
3. **HITL 数据面闭环**：页面按统一事件 envelope 追加写入宿主存储；Agent 用一条 CLI 命令按 session / 时间 / 版本过滤拉取。
4. **协议先行**：CLI↔daemon 的 loopback HTTP/JSON API 即协议本体，day one 带版本标识；未来 `--target` 指向 self-host/云端时协议不变。
5. **单端口统一入口**：全部页面与 API 收敛到一个 daemon、一个端口。

### 非目标（v0 明确不做）

- subscribe/callback 推送机制（协议留占位）。
- 内建 HITL 语义类型（表单提交/评分/批注等类型化查询；payload 由页面自理）。
- 反向代理活的 dev server（vite dev 等；仅静态产物 + SPA fallback）。
- 局域网访问与鉴权（仅 bind 127.0.0.1；协议为 remote target 预留身份字段占位）。
- 通用公网托管 / CDN / 多租户 SaaS；页面生成与设计能力本身。

## 3. 用户与场景

- **主用户**：开发者与本地协作者。
- **程序化用户**：各类 coding agent / skill（HITL skill、设计预览 skill、feature-tracker 等），既是页面提供方也是数据消费方。

典型场景：

1. **设计预览**：skill 生成页面 → `pageville publish` → 得到 `http://127.0.0.1:<port>/design-preview/`，迭代 N 次后仍可访问任一历史版本对比。
2. **HITL 回流**：HITL 页面托管在 Pageville 上，用户在页面里评分/批注 → 页面把事件 POST 进宿主 → 用户对 Agent 说「看下我刚填的」→ Agent `pageville events pull --page X --session Y` 一条命令拿到数据。
3. **工具页常驻**：feature-tracker 状态页等工具页有稳定 URL，不再每次换端口。

## 4. 产品原则

1. **一条命令可驱动**是硬性易用性门槛（CLI 首次调用自动拉起 daemon）。
2. **不可变快照是唯一真相**；`latest` 只是路由层指针，永不进入事件数据。
3. **契约最薄化**：宿主只定义信封，payload 语义由页面自理。契约越薄，接入面越大。
4. **协议先行 ≠ 仪式先行**：loopback API 本身就是协议，不做 SDK 矩阵、不做 OpenAPI 代码生成仪式。
5. **local-first**：默认零配置、零鉴权、仅本机；一切远端能力通过同一协议在后续版本扩展。

## 5. v0 范围

### 5.1 托管路由

| 路由 | 语义 |
|------|------|
| `GET /{page}/{version}/...path` | 钉住访问指定不可变快照内的文件 |
| `GET /{page}/latest/...path` | 解析到当前最新快照（等价下行） |
| `GET /{page}/...path` | ≡ `/{page}/latest/...path`（省略版本段默认 latest） |

- `{page}` 为语义化页面名（slug 规则：`[a-z0-9-]+`，与保留字 `api`、`latest` 冲突时拒绝注册）。
- `{version}` 为快照 id（内容寻址派生，见 §7），`latest` 是唯一的路由别名。
- SPA fallback：页面可在发布时声明 `spa: true`，未命中文件路径时回落到该快照的 `index.html`。
- MIME 按扩展名推断；未知类型 `application/octet-stream`。

### 5.2 版本化（Q-002 = A + 默认 latest）

- 每次 `publish` 产生一个**不可变快照**；快照内容与 id 永不改变。
- `latest` 指针切换必须**原子**（SQLite 事务或同文件系统原子 rename）：任何时刻请求要么读到旧版全貌、要么新版全貌，不存在半新半旧。
- 历史快照永远可回看；v0 不自动 GC（见 §12 开放问题）。

### 5.3 HITL 数据面（Q-003 = B，统一事件 envelope）

- 页面通过 HTTP API 向宿主**追加写**事件（append-only，不可修改不可删除）。
- 宿主定义信封字段；`payload` 为任意 JSON，语义由页面自理。
- envelope 的 `version` 字段**永远记录快照 id 本身**，即使页面经 `/{page}/` 或 `/{page}/latest/` 访问。宿主在服务页面时注入真实快照 id（见 §8 API）。
- Agent CLI 按 page（必选）+ session / 时间范围 / 版本（可选）过滤拉取。

### 5.4 CLI 最小命令集

```text
pageville publish <dir> --page <slug> [--spa]     # 发布快照，输出快照 id 与 URL
pageville pages list                               # 列出页面及最新版本
pageville versions list --page <slug>              # 列出某页面全部快照
pageville events pull --page <slug>                # 拉取事件（NDJSON 到 stdout）
    [--session <id>] [--since <ts>] [--until <ts>] [--version <snapshot-id>]
pageville daemon status|start|stop                 # daemon 生命周期（start 幂等）
pageville --target <url> ...                       # 协议端点切换（v0 默认 loopback，接受参数）
```

- `publish`、`pages`、`versions`、`events` 和 `daemon start` 首次调用时若 daemon 未运行则自动拉起；`daemon status` 只探测状态，`daemon stop` 只请求退出。auto-start 必须幂等（并发调用只产生一个 daemon）。
- 输出面向程序化消费：`--json` 全局旗标输出结构化结果；`events pull` 默认 NDJSON。

### 5.5 协议 / `--target`

- CLI 与 daemon 之间的 loopback HTTP/JSON API 即协议本体，路径前缀 `/api/v0/` 承载协议版本标识（论坛硬约束）。
- `--target` v0 仅接受并校验参数（默认 `http://127.0.0.1:<port>`），行为上只支持 loopback；协议报文预留 `identity` 占位字段（v0 恒空），供未来 remote target 鉴权使用。

## 6. 架构（O1：一体化 Rust daemon）

单一 `pageville` 二进制，两种运行形态（CLI 子命令 / daemon 进程）：

```text
┌─────────────────────────── pageville（单二进制）───────────────────────────┐
│                                                                            │
│  CLI（publish/pull/...) ──loopback HTTP/JSON (/api/v0/)──▶ daemon (axum)  │
│                                                             │              │
│                        ┌────────────────────────────────────┼─────────┐    │
│                        │ 静态托管                            │ 数据面  │    │
│                        │ /{page}/[{version}|latest]/...     │ /api/v0 │    │
│                        └───────────┬────────────────────────┴────┬────┘    │
│                                    ▼                             ▼         │
│                     文件系统 CAS 快照库                    SQLite          │
│                     ~/.pageville/objects/            ~/.pageville/         │
│                     （内容寻址，去重）                pageville.db          │
│                                                （页面/快照元数据 + 事件）   │
└────────────────────────────────────────────────────────────────────────────┘
```

- **HTTP 层**：axum；bind `127.0.0.1`，端口默认固定（如 7777，可配置），冲突时报错并提示。
- **快照存储**：文件系统 CAS。对象按内容 hash（blake3）落盘于 `objects/`，快照 manifest 记录「相对路径 → 对象 hash」映射；同内容跨版本零额外成本。
- **元数据与事件**：嵌入式 SQLite（WAL 模式）。数据库包含 pages、snapshots、latest 指针、events 四类表；latest 切换走事务。
- **协议同构**：daemon 对 CLI 暴露的 API 与未来 remote API 是同一套（`--target` 换 base URL 即可）。
- **Fallback（O2）**：若实现中 daemon 生命周期被实证过重（auto-start 幂等或崩溃一致性无法以合理成本满足），退化为文件系统真相 + 按需服务进程，保持 CLI 命令面不变。

## 7. 数据模型

### 7.1 快照（snapshot）

```text
snapshot_id : 快照内容派生 id（manifest 的 blake3 hash，取前 12 位十六进制展示）
page        : 页面 slug
created_at  : RFC 3339 时间戳
spa         : bool（SPA fallback 开关，随快照固化）
manifest    : { "<relative-path>": "<object-hash>", ... }
```

- `snapshot_id` 由内容派生 → 完全相同的重复发布天然幂等（同 id）。
- 快照一经写入不可变。发布顺序是先写全部对象与 manifest，再在事务中登记 snapshot 并切 latest；崩溃时最坏只丢失未完成的发布，不产生半新半旧。

### 7.2 latest 别名

- 每个 page 一个 `latest → snapshot_id` 指针，存 SQLite，事务内更新。
- 仅存在于路由/查询层；任何持久化事件数据不得引用 `latest`。

### 7.3 事件 envelope

```jsonc
{
  "event_id":  "evt_...",           // 宿主生成，全局唯一，单调可排序（如 ULID）
  "page":      "design-preview",    // 页面 slug（宿主校验存在）
  "version":   "a1b2c3d4e5f6",     // 快照 id 本体，永不为 "latest"
  "session":   "sess_...",          // 会话标识（页面提供；宿主提供获取途径，见 §8）
  "ts":        "<RFC3339 timestamp>",      // 宿主接收时间（RFC 3339，权威时间戳）
  "identity":  null,                 // v0 恒空；remote target 身份占位
  "payload":   { /* 任意 JSON，页面自理 */ }
}
```

- `event_id`、`ts` 由宿主生成（权威、可排序）；`page`/`version`/`session` 由写入方提供、宿主校验；`payload` 不校验内容。
- append-only：无更新、无删除 API。

## 8. CLI 与 API 面（最小集）

### 8.1 HTTP API（`/api/v0/`，即协议）

| 方法 & 路径 | 用途 | 说明 |
|-------------|------|------|
| `POST /api/v0/pages/{page}/snapshots` | 发布快照 | body 为 JSON `{files: {relative_path: base64_content}, spa: bool}`；服务端写入 CAS 对象与 manifest，成功返回 `snapshot_id` 并原子切 latest |
| `GET /api/v0/pages` | 列页面 | 含各页 latest 指针 |
| `GET /api/v0/pages/{page}/snapshots` | 列快照 | 时间倒序 |
| `POST /api/v0/pages/{page}/events` | 页面追加写事件 | body：`{version, session, payload}`；返回完整 envelope |
| `GET /api/v0/pages/{page}/events` | 过滤拉取事件 | query：`session` / `since` / `until` / `version`；NDJSON 流式返回 |
| `GET /api/v0/health` | daemon 健康与协议版本 | CLI auto-start 探测用 |

### 8.2 页面侧写入约定

- 被托管页面内可直接 `fetch("/api/v0/pages/{page}/events", {method:"POST", ...})`（同源，无 CORS 问题）。
- 宿主在服务页面时通过响应头 `X-Pageville-Page` / `X-Pageville-Version` 暴露真实快照 id；另提供 `GET /api/v0/context?path=...` 兜底，页面据此填写 envelope 的 `version` 字段，确保经 `latest` 访问时也记录快照 id 本体。
- `session` 由页面自定（建议：页面加载时生成并存 sessionStorage）。

### 8.3 CLI（见 §5.4）

CLI 是协议的第一个客户端，不走任何私有通道。全部功能经 `/api/v0/` 完成，这是「协议先行」的执行保证。

## 9. 验收标准（v0 Definition of Done）

以下每条均可由命令验证（对应 feature tracker 任务 gate）：

1. **发布闭环**：`pageville publish <dir> --page demo` 一条命令完成发布并输出 URL；`curl http://127.0.0.1:<port>/demo/` 返回页面内容。
2. **版本语义**：发布两个不同版本后，`/demo/` 与 `/demo/latest/` 内容一致且为 v2；`/demo/{v1-id}/` 仍返回 v1 内容；重复发布相同内容得到相同 snapshot_id。
3. **原子 latest**：发布过程中并发请求 `/demo/`，任一响应要么全旧要么全新（无 404 / 混合内容窗口）。
4. **事件闭环**：向 `/api/v0/pages/demo/events` POST 事件后，`pageville events pull --page demo --session <s>` 能按 session / 时间 / 版本过滤拉回，envelope 的 `version` 为快照 id 本体（即使写入方经 `/demo/` 访问）。
5. **daemon 生命周期**：daemon 未运行时数据命令与 `daemon start` 自动拉起；并发调用只产生一个 daemon 实例；`pageville daemon status` 可探测状态，`pageville daemon stop` 干净退出。
6. **SPA fallback**：`--spa` 发布的页面，未命中路径回落 `index.html`；非 SPA 页面未命中返回 404。
7. **协议面**：所有 CLI 功能均经 `/api/v0/` HTTP 完成；`--target http://127.0.0.1:<port>` 显式传入时行为不变。

## 10. Out of scope / 后续阶段

| 阶段 | 内容 |
|------|------|
| 后续 | 具名渠道（draft/stable 等，Q-002 B 档的增量）；subscribe/callback 事件推送 |
| 后续 | 内建 HITL 语义类型与按类型查询（Q-003 C 档） |
| 后续 | dev-server 反向代理 adapter（Q-004 C 项） |
| 后续 | 局域网访问、token/身份鉴权、remote `--target` 实际启用（Q-005 B/C 项） |
| 后续 | self-host / 云端部署形态；快照 GC 命令实装 |

## 11. 风险与对策

| 风险 | 触发条件 | 对策 |
|------|----------|------|
| daemon 生命周期过重 | auto-start 幂等 / 崩溃一致性成本超预期 | 触发 O2 fallback：文件系统真相 + 按需服务进程，CLI 命令面不变 |
| 协议被外部 skill 依赖后锁死 | 首个外部消费者出现 | `/api/v0/` 版本前缀 day one 生效；破坏性变更走 `/api/v1/` |
| 半新半旧的 latest | 切换非原子 | 硬约束：SQLite 事务切指针 + 先写对象后登记；验收标准 #3 专项验证 |
| 存储无限增长 | 长期使用不清理 | CAS 去重压低成本；v0 显式声明「不自动删」，预留 `gc` 命令语义占位 |

## 12. 开放问题（显式标注，不阻塞 v0）

1. **GC 与保留策略**：`pageville gc` 的用户可见语义（按页面保留 N 版？按时间？事件随快照删还是独立保留？）尚未确定。v0 只承诺「永不自动删」，命令签名预留。
2. **daemon 生命周期集成**：是否提供 launchd/systemd 集成实现开机自启与崩溃自动重启；v0 仅 CLI auto-start。
3. **协议版本纪律细则**：`/api/v0/` 内的字段级兼容策略（只增不改？envelope 扩展字段规则）尚未确定，须在首个外部 skill 依赖前定稿。
4. **端口固定策略**：默认端口取值与冲突时的行为（报错 vs 自动递增）仍待确定。v0 先报错并提示，端口可配置。
