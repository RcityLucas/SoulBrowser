# SoulBrowser 模块文档

> 本目录按模块梳理 SoulBrowser 工作区，聚焦每组 crate 的职责、入口文件、常用命令及关键依赖。

> 若需更细致的 API/流程解析，请参考同目录下的 [`module_deep_dive.md`](./module_deep_dive.md)。

## 目录
1. [CLI 壳层](#cli-壳层)
2. [Kernel / Runtime](#kernel--runtime)
3. [动作与调度链路](#动作与调度链路)
4. [多模态感知栈](#多模态感知栈)
5. [控制面与存储](#控制面与存储)
6. [治理、权限与集成](#治理权限与集成)
7. [Agent / LLM 支撑](#agent--llm-支撑)
8. [观测与运维](#观测与运维)

---

- **辅助阅读**：本文件概述，`module_deep_dive.md` 提供 API/流程详解。

## CLI 壳层
- **路径**：`src/cli/`、`src/bin/parser_scaffold.rs`
- **核心文件**：
  - `app.rs`（CLI 启动、metrics/logging）
  - `runtime.rs`（配置加载、env override）
  - `context.rs`（懒加载 `AppContext`）
  - `commands.rs` / `dispatch.rs`（命令枚举 + 分发）
- **命令模块**：`serve`、`gateway`、`perceive`、`chat`、`scheduler`、`perceiver`、`policy`、`timeline`、`artifacts`、`console`、`memory`、`config`、`info`、`run_bundle` 等。
- **集成点**：通过 `CliContext` 共享 `Config`/`AppContext`/输出目录；metrics server 由 CLI 负责启动。
- **延伸**：详见 `module_deep_dive.md` §1（启动流程、调试技巧）。

## Kernel / Runtime
- **路径**：`crates/soulbrowser-kernel/`
- **关键模块**：`kernel.rs`（Serve/Gateway）、`runtime.rs`（tenant/LLM/cache/并发）、`app_context.rs`（依赖聚合）、`perception_service.rs`（Chrome/Perceiver）、`metrics.rs`、`gateway/` / `server/`。
- **输出接口**：`Kernel::serve`、`Kernel::gateway`、`AppContext::new/get_or_create`、`ServeOptions`、`GatewayOptions`。
- **要点**：统一的 `ServeState` 注入 axum router；`GatewayPolicy` 处理 token/IP；`AppContext` 使用缓存避免重复初始化。
- **延伸**：详见 `module_deep_dive.md` §2（Serve 流程、配置/env 影响）。

## 动作与调度链路
- **Action Crates**：
  - `action-primitives`：navigate/click/type/select/scroll/wait 以及等待策略。
  - `action-locator`：候选定位、策略、愈合（healer）。
  - `action-gate`：条件校验、证据、验证器。
  - `action-flow`：执行策略、调度器、flow types。
  - `soulbrowser-actions`：上述模块的汇总导出。
- **Scheduler/Registry**：
  - `crates/registry`：会话、页面、路由 (`ExecRoute`) 管理。
  - `crates/scheduler`：`Dispatcher`、`Orchestrator`、`SchedulerRuntime`、`ToolExecutor` 接口。
  - `state-center`：记录调度成功/失败、感知事件，用于 CLI/Console。
- **CLI 对应**：`chat --execute`、`scheduler` 命令、Serve Console 的执行流。
- **延伸**：`module_deep_dive.md` §3（Action/Scheduler 数据流、调试手段）。

## 多模态感知栈
- **结构感知**：`perceiver-structural`（API、策略、缓存、评审、快照/diff）。
- **视觉感知**：`perceiver-visual`（截图、OCR、视觉 diff/metrics、缓存）。
- **语义感知**：`perceiver-semantic`（分类、language、keywords、summary）。
- **融合**：`perceiver-hub`（`PerceptionHubImpl`）、`perception_service.rs`。
- **网络信号**：`network-tap-light`（请求窗口、安静期统计）。
- **CLI 对应**：`perceive` 命令、`serve` Console 预览、`chat` 规划时的页面理解。
- **延伸**：`module_deep_dive.md` §4（PerceptionJob 结构、Pooling 策略）。

## 控制面与存储
- **事件与时间线**：`event-store`（冷热环、append API）、`l6-timeline`（导出/Replay）、`snapshot-store`。
- **策略中心**：`policy-center`（snapshot/override/watch）、`permissions-broker`（CDP 权限）、`policy defaults`。
- **内存/记录**：`memory-center`（轻量记忆）、`state-center`（运行事件）。
- **存储 & Integration**：`integration-soulbase`（文件/内存存储、Auth、Tool Manager 提供者）。
- **CLI 对应**：`timeline`、`artifacts`、`memory`、`policy`、`config`、`info`。
- **延伸**：`module_deep_dive.md` §5（Event Store/Policy/Memory 细节）。

## 治理、权限与集成
- **CDP 与浏览器**：`cdp-adapter`（Chrome 路径检测、Adapter 结构）、`stealth`。
- **权限/扩展**：`permissions-broker`（策略 + CDP 透传）、`extensions-bridge`（MV3 通道脚手架）。
- **L7 Adapter**：`l7-adapter`（HTTP Router、Guard、Idempotency、事件转发）、`l7-plugin`（插件运行时、沙箱、策略、审计）、`l7-plugin`、`plugins` 配置。
- **隐私与观测**：`l6-privacy`（遮罩/截图脱敏）、`l6-observe`（导出/trace）。
- **CLI 对应**：`gateway`、`serve --surface gateway`、未来 gRPC/WebDriver。
- **延伸**：`module_deep_dive.md` §6（CDP/Network Tap/L7 Adapter/插件/隐私）。

## Agent / LLM 支撑
- **`agent-core`**：Agent 请求/上下文/计划结构、Planner/Validator、LLM Provider 接口。
- **Kernel Agent 模块**：`soulbrowser-kernel::agent`、`chat_support`、`plan_payload` 等帮助 `chat` 命令执行、落地 plan/flow；`agent::message_manager` 负责 BrowserUse 风格 `<initial_user_request>`/`<follow_up_user_request>`/`<read_state_x>` 历史块，`chat` CLI 会自动注入这些 metadata 供 LLM 规划/重规划使用。
- **LLM 缓存**：`soulbrowser-kernel::llm` + runtime LLM cache 目录控制。
- **CLI 对应**：`chat` 命令（planner 选择、LLM provider override、执行报告/Artifacts 保存）。
- **延伸**：`module_deep_dive.md` §7（Agent 模型、Planner/LLM 执行链）。

## 观测与运维
- **Metrics**：`soulbrowser-kernel::metrics` 注册 scheduler/registry/cdp/plan/LLM 指标，`serve` 启动时监听 `metrics_port`。
- **State Center CLI**：`scheduler`, `perceiver`, `info` 命令读取 `state_center_snapshot`。
- **Artifacts & Console**：`artifacts.rs`、`console.rs`、Run Bundle (`chat --save-run`)。
- **日志**：`tracing` + `local.env` (`RUST_LOG`)，`humantime` 指标打印。
- **延伸**：`module_deep_dive.md` §8（Metrics/StateCenter/Artifacts 详解）。

---

> 若需更新模块职责或新增子系统，请同步修改本文件并在根 README 中添加链接。
