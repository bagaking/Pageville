# CURRENT — 执行状态与质量门禁

> meta：项目驱动文档。**任何关键节点（sprint 完成、重大变更、验收、与用户深入
> 讨论后）必须更新本文件。** 产品范围与验收标准的 SSOT 是
> [`docs/PRD/2026-05-01-pageville-host.md`](../docs/PRD/2026-05-01-pageville-host.md)；
> 北极星与选型见 [`PLAN.md`](PLAN.md)。

## 一、标准与原则

### 目录组织（遵循全局最佳实践）

- 本目录平铺，无子目录；`PLAN.md` / `CURRENT.md` 固定名，其余语义化命名。
- **SSOT**：写之前先搜项目内是否已有，用引用代替抄写。PRD 已写的内容不复制。
- Milestone-Sprint 文档单独成文；执行中发现测试不达预期 / 回归 / 重构 / 计划偏差，
  新建 mini-sprint（如 `M01_S01.003.FIX.xxx.md`），**同时联动更新本文件**。
- 一般同时最多一个"活跃中"的 mini-sprint。

### 项目个性化原则

1. **ponytail 基线**（用户两次显式加载）：YAGNI、删优于加、最短可用 diff。
   豁免项不得简化：信任边界校验、防数据丢失的错误处理、安全措施。
   → [`OPINIONS_001`](OPINIONS_001.quality_bar.md) O-006
2. **测试必须能红**：每条断言要能回答"被测逻辑坏了它会红吗"。关键断言做
   mutation 验证。 → O-001 / O-002
3. **拒绝静默错误答案**：解析失败在边界处拒绝（4xx），不在比较函数里兜底。 → O-003
4. **CLI 退出码是契约**：失败必须 stderr + 非零退出。新增 HTTP 调用点走
   `get_text()`。 → O-004
5. **数据完整性不接受"存在即正确"**：幂等写要问"存在但损坏怎么办"。 → O-005
6. **安全面是"浏览器可达"而非"网络可达"**。 → O-007
7. **注释里的并发模型必须与真实并发单元核对**（进程？线程？async task？），
   用压测证明而非注释声明。 → O-008
8. **验证退出码不经管道**；断定被测系统有问题前先排除测量方式有问题。 → O-009
9. 测试从 PRD / Milestone 目标出发，**不得反向为用例适配代码**；修复面向正确
   行为，不用 workaround 或测试桩。

## 二、当前执行状态

**里程碑 M01（v0 可用性与质量收敛）：进行中**

| Sprint | 内容 | 状态 |
| --- | --- | --- |
| M01_S01 | v0 功能闭环（发布/版本/事件/CLI/Atlas） | 已完成（见 git 历史） |
| [M01_S01.001.FIX](M01_S01.001.FIX.security_and_verification.md) | 安全加固 + 虚假验收修复 | **已验收** |
| [M01_S01.002.FIX](M01_S01.002.FIX.data_integrity_and_cli.md) | 数据完整性 + CLI 契约 | **已验收** |
| [M01_S01.003.FIX](M01_S01.003.FIX.adversarial_probe_findings.md) | 并发/文件系统/生命周期三面探测结论 | **已验收** |
| [M01_S01.004.FIX](M01_S01.004.FIX.cas_growth_and_publish_ordering.md) | CAS 增长实测 + 发布顺序修复（GC 决策） | **已验收** |
| [M01_S01.005.FIX](M01_S01.005.FIX.retention_decision_and_store_visibility.md) | 保留策略实测否决 + 存储体积可见化 | **已验收** |

### 最近一次全量质检结果

| 项 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets --locked -- -D warnings` | 0 警告 |
| `cargo test --locked` | 10/10 |
| `node scripts/npm/check.js` | 通过 |
| `bash scripts/verify/t008-acceptance.sh` | 12/12，**退出码 0（未经管道实测）** |
| 60 线程并发发布相同内容 | 0 个 5xx，无残留 tmp |

### 已知未决

- 无 `project_plan` 之外的阻塞项。
- **CAS GC：已决策不做**，理由见 [M01_S01.004](M01_S01.004.FIX.cas_growth_and_publish_ordering.md)
  （正常运行可回收垃圾实测为 0；真正的垃圾源已在上游消除）。
- **保留策略（keep=N）：已决策不做**，理由见
  [M01_S01.005](M01_S01.005.FIX.retention_decision_and_store_visibility.md)
  （常见负载回收 0.2%，代价是 pinned URL 静默返回 200 配错内容）。已改为把体积
  通过 `daemon status` 暴露给所有者。真要做的三个必要条件记录在该文件末尾。
- CI 尚未在真实 push 上触发过。
- 许可证 MIT（依赖树核查无传染性协议，见 PLAN.md）。

## 三、整体质检步骤（每次重大变更后执行）

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
node scripts/npm/check.js
bash scripts/verify/t008-acceptance.sh   # 任一切片失败会点名并非零退出
```

**附加动作**（不可省略，来源见括号）：

1. 新增回归断言后，把被测代码改回旧实现，确认断言会红，再改回来。
   （O-002，来源 M01_S01.002）
2. 新增 HTTP 调用点后，验证失败路径 stderr + 非零退出。（O-004，来源 M01_S01.002）
3. 改动发布/存储路径后，跑一次截断注入：截半 CAS 对象 → 重新发布 → 断言恢复。
   （O-005，已固化进 `t002`）
4. 改动响应头或路由后，确认 `nosniff` / CSP / `charset` 仍覆盖所有被服务的类型。
   （O-007，已固化进 `t011`）
5. 改动时间过滤后，确认三种 RFC3339 拼写等价、垃圾输入 400。
   （O-003，已固化进 `t012`）
6. 改动路径校验后，确认含 `..` 子串的合法名可发布、真实穿越仍 400、目录式
   URL（尾斜杠）仍解析为 index。（F-1，已固化进 `t010`/`t012`）
7. 改动 CAS 写入或 spawn 路径后，跑 60 线程并发发布相同内容，断言 0 个 5xx。
   （O-008，来源 M01_S01.003）
8. 改动发布校验顺序后，确认被拒的发布不产生任何对象（校验先于写盘）。
   （O-010，已固化进 `t002`）
9. 给既有命令加输出前，先 grep 谁在消费它；`daemon status` 第一行是裸词契约，
   明细只能追加在后续行。（O-012，已固化进 `t007`/`t011`）

## 四、文档索引

| 文件 | 内容 |
| --- | --- |
| [PLAN.md](PLAN.md) | 北极星、用户原话、技术选型、许可证决策 |
| [OPINIONS_001.quality_bar.md](OPINIONS_001.quality_bar.md) | 质量基线与审美（O-001 ~ O-012） |
| [M01_S01.001.FIX...](M01_S01.001.FIX.security_and_verification.md) | 安全加固与虚假验收修复 |
| [M01_S01.002.FIX...](M01_S01.002.FIX.data_integrity_and_cli.md) | 数据完整性与 CLI 契约 |
| [M01_S01.003.FIX...](M01_S01.003.FIX.adversarial_probe_findings.md) | 三面对抗探测结论 |
| [M01_S01.004.FIX...](M01_S01.004.FIX.cas_growth_and_publish_ordering.md) | CAS 增长实测与 GC 决策 |
| [M01_S01.005.FIX...](M01_S01.005.FIX.retention_decision_and_store_visibility.md) | 保留策略否决与体积可见化 |
| [../docs/PRD/](../docs/PRD/2026-05-01-pageville-host.md) | **产品范围与验收标准 SSOT** |
