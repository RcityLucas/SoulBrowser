# L0 运行与适配 · 实现路线图

本文整理如何将项目从当前脚手架推进到“接入真实 L0 事件源 + 监控链路 + 验收”的落地方案，便于统一内核(L1)与 L0 之间协同开发。

## 0. 背景现状

- `crates/cdp-adapter`、`crates/network-tap-light` 仅提供接口和简单内存实现，尚未连接真实 Chromium/CDP 或网络探针。
- L0 文档（`L0 运行与适配/01-05`) 包含详细设计与规约，但没有代码落地。
- L1 侧的 `l0_bridge` 已可订阅 `cdp_adapter::RawEvent` 与 `network_tap_light::NetworkSummary`，并将其转化为 Registry/State Center 事件，用于端到端测试。

## 1. CDP 适配层推进计划

1. **真实 transport 封装**
   - 选型：Chromiumoxide / Chrome DevTools Protocol over WebSocket。
   - 实现 `crates/cdp-adapter/src/transport.rs`：
     - 启动浏览器进程或连接远端 Chrome。
     - 维护 session/page/frame 映射；处理 reconnect、自愈逻辑。
     - 提供 `next_event()` 和 `send_command()` 异步接口。

2. **命令与事件聚合**
   - 按文档 8 个能力面实现 `commands.rs`，编写导航/点击等基础命令；
   - `events/agg.rs` 负责将原始事件去抖/聚合为 `RawEvent`。
   - 为 `RawEvent` 提供环形缓存和过滤订阅（`events/bus.rs`）。

3. **配置与 Feature Flag**
   - 与 `soulbase-config` 对接，支持多实例、不同启动参数、策略开关。

4. **测试策略**
   - 单元：使用 chromiumoxide 的 mock / WebSocket 回环。
   - 集成：在 CI 启动 headless Chrome，跑 smoke tests；
   - 故障注入：WebSocket 断链、页面 crash、命令超时。

## 2. 网络轻探针（Network Tap）

1. **数据采集**
   - 从 CDP `Network.*` 事件或 Soulbase tap 获取请求/响应统计。
   - `NetworkTapLight` 提供enable/disable、snapshot、summary 发布。

2. **事件发布**
   - 通过 L0 bridge 将 `NetworkSummary` 送入 Registry，并写 State Center。

3. **指标**
   - 连接 Soulbase-observe/metrics（若可用）或 Prometheus exporter。

## 3. L0 ↔ L1 事件桥接

当前 `src/l0_bridge.rs` 已支持：

- 从 CDP RawEvent 创建/关闭/聚焦 Page & Frame；
- 从网络摘要更新 `PageHealth`，并触发 `RegistryAction::PageHealthUpdated`；
- 将事件附带 action/task 信息写入 State Center。

需要进一步完成：

- 真实 transport 接入后，替换测试事件为实时事件；
- 处理多 session/page 映射（目前默认会话 `cli-default`）。

## 4. 监控与指标

1. **Scheduler metrics**：已有内存计数器；下一步接入 Soulbase-observe 或 Prometheus endpoint。
2. **Registry/StateCenter 事件**：借助 `docs/l1_operations.md` 指南输出快照，监控 Page/PageHealth。
3. **Logs/Tracing**：统一使用 `tracing`，配置到 Soulbase-observe。

## 5. 验收与测试

参见 `docs/l1_acceptance_checklist.md`，关键步骤包括：

- 功能：CDP 命令、事件聚合、网络统计、State Center 快照。
- 性能：调度吞吐、CDP 调用 P95、恢复策略。
- 故障注入：cancel、timeout、断链自愈。
- 安全：策略 override 白名单、数据脱敏。

### 自动化脚本建议

1. **端到端测试 (`tests/l1_e2e.rs`)**：可扩展为多 URL/多任务场景；配合真实 L0 数据后，移除 `sleep`，改为事件监听。
2. **负载/性能测试**：Benchmark harness（Tokio + criterion 或 Soulbase 自带工具）。
3. **监控报警**：定义指标阈值，配置在运维平台。

## 6. 阶段里程碑

1. **Milestone A**：CDP transport + 基础命令完成，RawEvent 输出；
2. **Milestone B**：Network tap 整合 + Registry 更新 + State Center 事件；
3. **Milestone C**：Scheduler 指标 + Cancel 流程全闭环；
4. **Milestone D**：验收 checklist 跑通（性能、故障注入、安全测试）；
5. **Milestone E**：文档/运维手册完备，CI 加入端到端和离线测试。

## 7. 依赖与风险

- 需确认 Chrome/Chromium 版本、运行环境、权限。
- L0 与 Soulbase 组件的接口稳定性（auth、observe、storage）。
- 监控平台及报警策略；需运维团队配合。

这份路线图可放入项目议程，逐项分解到具体任务。若有新的外部约束（例如现成的 L0 服务、不同的指标平台），可在对应的章节补充。

