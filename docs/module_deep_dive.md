# SoulBrowser 模块深度解析（2024-）

> 该文档从架构视角纵向拆解每个模块：职责、核心结构、主要代码位置、数据流、调试要点以及与其他层的耦合点。可作为开发/排障前的必读材料。

## 1. CLI Shell 层
**范围**：`src/cli/`、`src/bin/`

### 1.1 核心结构
- `CliArgs` (`env.rs`)：定义全局参数（`--config`/`--log-level`/`--metrics-port`/`--output`），并嵌套 `Commands`。
- `Commands` (`commands.rs`)：枚举 20+ 子命令，兼容退役命令（`run`/`record`/`replay` 等，直接 `bail!()` 提示新流程）。
- `CliContext`：携带 `Config`、配置文件路径、metrics 端口以及 `OnceCell<AppContext>`，确保命令共享 runtime。
- `LoadedConfig` (`runtime.rs`)：封装配置文件读取+默认路径（`~/.config/soulbrowser/config.yaml`）。

### 1.2 启动流程
1. `src/main.rs` 调 `cli::app::run()`。
2. `app.rs`
   - 调 `load_local_env_overrides()` 解析 `config/local.env`，支持引号/转义。
   - `CliArgs::parse()`；根据 `--debug` 或 `--log-level` 初始化 tracing。
   - `metrics::spawn_metrics_server(port)` 启动 Prometheus 端点。
   - `load_config()` 从 CLI 指定或默认路径读取 YAML，若不存在创建默认 `Config`。
   - `apply_runtime_overrides()` 根据配置写入 `SOUL_POLICY_PATH`/`SOUL_STRICT_AUTHZ` 等 env。
   - 构造 `CliContext`，交给 `dispatch()`。
3. `dispatch.rs` pattern-match `Commands`，调用各 `cmd_*` 实现。

### 1.3 命令详解
- `serve`：构建 `ServeOptions`（host/port/tenant/token/IP/表面类型/共享 session/LLM cache），交给 `Kernel::serve`。
- `gateway`：组装 `GatewayOptions`（HTTP/gRPC/WebDriver 监听 + policy/plan），调用 `Kernel::gateway`。
- `perceive`：根据 `--all`/`--visual`/`--semantic`/`--structural` 组合模式，支持 `--ws-url`/`--chrome-path`/`--headful`/`--timeout`/`--insights`，输出 JSON + screenshot。
- `chat`：支持 prompt 字符串/文件、constraints、planner/LLM overrides、`--execute`、最大重试/重规划、artifact/run bundle 持久化；内部调用 `agent_core` + `chat_support`。
- `scheduler` & `perceiver`：对 `StateCenter` 数据进行过滤/格式化（`text/json` 或 `table`）。
- `policy`：`show` 命令组合策略视图+StateCenter/Scheduler 概要；`override` 写入运行时 override，支持 TTL。
- `timeline`：拉取存储 (`StorageManager`) + `event-store`，支持 `records|timeline|replay` 视角以及时间范围选择。
- 其他如 `artifacts`、`console`、`memory`、`info`、`config` 等各司其职。

### 1.4 调试建议
- `RUST_LOG=soulbrowser=debug` + `--debug` 观察启动/Serve 日志。
- `--metrics-port 0` 暂时关闭 metrics 以排查端口冲突。
- Run bundle 文件 (`chat --save-run`) 可用 `console --serve` 快速复现 UI。

## 2. Kernel & Runtime
**范围**：`crates/soulbrowser-kernel`

### 2.1 梯度结构
- **入口**：`Kernel::serve`、`Kernel::gateway`、`Kernel::build_app_context`。
- **状态**：`ServeOptions`、`RuntimeOptions`、`RuntimeHandle`、`ServeState`（含 websocket URL、LLM cache、RateLimiter、AppContext RwLock）。
- **AppContext**：集中化 Storage、Auth、Session Service、Registry、Scheduler、Policy Center、Plugin Registry、Memory Center、State Center、Self Heal、Manual Override 等。
- **PerceptionService**：Chrome/CDP 共享 session、截图、日志/insight；受 `SOULBROWSER_DISABLE_PERCEPTION_POOL` 控制。
- **Gateway/Server 模块**：使用 `axum` 构建 HTTP surface，并套上 `gateway_ip_middleware`/`gateway_auth_middleware`。

### 2.2 Serve/Gateway 流程
1. `Kernel::serve(options)`
   - `start_runtime()`：准备 tenant 目录（`tenant_storage_path`）、LLM cache dir、RateLimiter（`SOUL_CHAT_CONTEXT_LIMIT/WAIT_MS`）、`AppContext`。
   - `build_serve_auth_policy()`：合并 CLI/ENV token 与 IP allowlist；若为空则警告。
   - `build_api_router_with_modules()` 选择 console shell + API modules；`ServeSurfacePreset::Gateway` 仅包含 API。
   - `start_http()` 绑定 listener，打印访问地址 + Chrome 模式。
2. `Kernel::gateway(options)` 直接调用 `gateway::run_gateway`（整合 L7 adapter + policy/mcp/gRPC 预留）。

### 2.3 AppContext 细节
- `IntegrationProvider`（默认 `integration-soulbase`）决定 Storage/Auth/Tool Manager 的实现。
- 策略加载顺序：`config.policy_paths` > `config/policy.yaml` > `default_snapshot()`。
- 通过 `DashMap<ContextCacheKey, Weak<AppContext>>` 缓存，key = tenant + storage path + policy hash，避免重复构建。
- 各组件初始化顺序：Storage → SessionService → AuthManager → ToolManager → PolicyCenter → Registry/RegistryBus → SchedulerRuntime/SchedulerService → StateCenter → PluginRegistry → Memory/SelfHeal/ManualOverride。

### 2.4 配置/Env 影响点
- `strict_authorization` 配置或 Serve Auth token 会设置 `SOUL_STRICT_AUTHZ`。
- `policy_paths`/`SOUL_POLICY_PATH` 影响策略加载路径。
- `SOUL_SERVE_SURFACE` 允许在不改 CLI 的情况下切换 Console/Gateway。
- `SOULBROWSER_LLM_CACHE_DIR` / CLI `--llm-cache-dir` 控制 Planner 缓存目录。

## 3. 动作与调度链路
**范围**：`crates/action-*`, `crates/scheduler`, `crates/registry`, `crates/state-center`, `crates/soulbrowser-actions`

### 3.1 Action 层
- `action-primitives`：封装 6 大原语 + `WaitCondition`/`WaitTier`。
- `action-locator`：selector resolver + healer + strategies；可结合 perceiver 数据进行纠错。
- `action-gate`：执行前/后验证（IsVisible/IsEnabled/MatchesText 等），输出 `Evidence`。
- `action-flow`：Flow executor/strategies，负责 orchestrator 级重试；`types` 描述状态机。
- `soulbrowser-actions`：re-export 供其它 crate 使用。

### 3.2 Registry 与 Scheduler
- `Registry` trait：`session_create`, `page_open`, `route_resolve`, `frame_focus` 等；`ExecRoute` 包含互斥 key 以保护 frame。
- `SchedulerService`：`Dispatcher` 实现 + `Orchestrator`，控制 submit/cancel；与 `SchedulerRuntime` (queue/slot/priority) & `ToolExecutor` 协作。
- `state-center`：记录 `DispatchEvent`（actionId/tool/wait/run/error/pending/slots）、`PerceiverEvent`（resolve/judge/snapshot/diff）、`ScoreComponentRecord`。

### 3.3 计划执行链路
1. Agent Planner 产生 `AgentPlan`。
2. `action_flow::plan_to_flow` 转为 Flow 图。
3. `execute_plan` 调 `SchedulerService::submit`。
4. Orchestrator 分配 slot → ToolExecutor 执行（当前可替换 stub）→ 结果写 `StateCenter`。
5. CLI（`scheduler`/`chat`）或 Serve Console 读取这些事件，实时显示。

### 3.4 调试
- `soulbrowser scheduler --status failure --limit 20` 查看最近失败。
- `policy override scheduler.limits.global_slots` 快速模拟并发调整。
- `state_center_snapshot()` 可写入磁盘（若 `state-center` 持久化开启）供离线分析。

## 4. 多模态感知栈
**范围**：`crates/perceiver-*`, `crates/perceiver-hub`, `crates/network-tap-light`, `soulbrowser-kernel::perception_service`

### 4.1 Structural/Visual/Semantic Perceiver
- `StructuralPerceiver`：`resolve_anchor(_ext)`, `is_visible`, `is_clickable`, `snapshot_dom_ax`, `diff_dom_ax`；`ResolveOptions` 控制候选数、fuzziness；`PerceiverPolicyView` 管理缓存/评分策略。
- `VisualPerceiver`：`ScreenshotCapture` + `VisualPerceiverImpl`，可输出 `VisualMetrics`（平均对比度、viewport utilization）与 `VisualDiff`（高亮变化）。
- `SemanticPerceiver`：聚合分类器、语言检测、关键词提取、摘要，输出 `language`+置信度、`intent`、`summary`、`keywords`、`readability`。

### 4.2 PerceptionHub & Service
- `PerceptionHubImpl::perceive(route, options)` 并行执行结构/视觉/语义，输出 `MultiModalPerception`（包含 cross-modal insights）。
- `PerceptionService`
  - 构建 `PerceptionJob`（URL、模式开关、截图、ws/headful/cookies/script/timeout/viewport）。
  1. 决定 Chrome 模式（共享/独占 session），若设置 `--ws-url` 则直接连接现有调试端口。
  2. 调 `PerceptionHub` 获取 perception + screenshot；根据 `capture_screenshot` 决定是否写 bytes。
  3. 产出 `PerceptionFilePayload`（url + perception + logs + screenshot Base64）。
- CLI `perceive` & Serve Console overlay 复用此 service 输出。

### 4.3 扩展
- `network-tap-light` 可向感知层提供网络窗口统计（req/res/quiet/inflight）。
- 未来可在 `PerceptionJob` 注入 cookies/脚本以测试登录场景。

## 5. 控制面、存储与治理
**范围**：`event-store`, `l6-timeline`, `policy-center`, `permissions-broker`, `memory-center`, `integration-soulbase`

### 5.1 Event Store & Timeline
- 支持 append、batch、flush、tail/since/by_action、export_range、replay、stream。
- `HotWriter` + `ColdWriterHandle` 控制冷热分层，` redact::apply` 根据 policy 屏蔽敏感字段。
- `TimelineService` 结合 Event Store + State Center，导出 `records`（文本）、`timeline`（时间轴）、`replay`（用于回放工具）。

### 5.2 Policy & Permissions
- `InMemoryPolicyCenter`
  - `snapshot()` 返回 Arc；`apply_override()` 支持 TTL + watch 通知。
  - CLI `policy show --json` 输出策略 + Scheduler/StateCenter 概要。
- `permissions-broker`：
  - 解析 `PolicyFile` + `PermissionMap`；`pattern_matches` 支持通配；`AuthzDecision`（allow/deny/missing + TTL）。
  - `CdpPermissionTransport` 预留与 CDP 的交互接口。

### 5.3 Memory & Storage
- `MemoryCenter`
  - `store/list/remove_by_id/update`，支持 tag/note 正规化；可 `with_persistence(path)` 写入 JSON。
  - CLI `memory add/list/export/import/stats` 直接调用。
- `integration-soulbase`：
  - `create_storage_manager`: file-based（默认输出 `soulbrowser-output`）或 in-memory。
  - `create_auth_manager`: 根据 policy paths 初始化 `BrowserAuthManager`。
  - `create_tool_manager`: 注册默认浏览器工具。

## 6. 治理、插件与外部接口
**范围**：`cdp-adapter`, `network-tap-light`, `l7-adapter`, `l7-plugin`, `l6-privacy`, `l6-observe`, `stealth`

### 6.1 浏览器与网络
- `cdp-adapter`: 检测 Chrome 路径 (`SOULBROWSER_CHROME`/`which`/默认)，`CdpConfig` 记录 headless/user-data-dir/heartbeat；事件（`RawEvent`）描述 navigation/network 状态。
- `network-tap-light`: `NetworkTapLight::enable_page` + `register_event` 聚合窗口统计，通过 `SummaryBus` 广播。
- `stealth`: 预留反检测逻辑。

### 6.2 L7 Adapter & Plugin Runtime
- `l7-adapter`
  - `AdapterBootstrap` 注入 Policy/Dispatcher/Readonly Ports；`RequestGuard` + `AdapterPolicyHandle` 管理租户策略；`idempotency`/`trace`/`policy` modules 支持治理。
  - gRPC/WebDriver/MCP 模块目前为 TODO，但接口已定义。
- `l7-plugin`
  - `PluginManifest`, `Permissions`, `ProviderSpec`, `PluginRuntime`, `SandboxHost`, `PluginAuditEvent`, `HookCtx`；
  - `KillSwitch`、`PluginPolicyView`、`Trust` 等控制插件生命周期与权限。

### 6.3 隐私 & 观测
- `l6-privacy`: `apply_event/apply_export/apply_screenshot` 等函数执行脱敏；`RedactCtx`/`RedactScope` 控制策略；`PrivacyPolicyView` 可自定义规则。
- `l6-observe`: policy/metrics/tracing/exporter，方便 Serve/Gateway 安全输出观测数据。

## 7. Agent / LLM 支撑
**范围**：`agent-core`, `soulbrowser-kernel::agent`, `chat_support`

- `agent-core`
  - 模型：`AgentContext`, `AgentRequest`, `AgentPlan`, `AgentPlanStep`, `AgentToolKind`, `AgentWaitCondition`, `AgentValidation`。
  - Planner：`AgentPlanner`, `RuleBasedPlanner`, `PlannerOutcome`, `PlanValidator`（`PlanValidationIssue` 集合）。
  - LLM Provider：`LlmProvider`, `MockLlmProvider`；`PlannerSelection`/`LlmProviderSelection` 提供字符串解析。
  - `plan_to_flow`：把 Agent Plan 变为 action flow（含 `PlanToFlowOptions`/`PlanToFlowResult`）。
- `chat_support`
  - `build_chat_runner` 选择 planner/LLM provider + fallback reason；`FlowExecutionOptions` 控制执行参数。
  - `plan_payload` 提供 Web Console 所需 JSON。
- 执行链：prompt → `AgentRequest` → Planner（rule/LLM）→ Flow → `execute_plan` → `FlowExecutionReport` → artifacts/run bundle。
- CLI `chat` 支持 `--save-plan`、`--save-flow`、`--save-run`、`--artifacts-only` 等，便于离线分析。

## 8. 观测与运维
**范围**：`soulbrowser-kernel::metrics`, `state-center`, CLI `scheduler/perceiver/info`, `artifacts`, `console`

- **Metrics**
  - `register_metrics()`（One-time）注册 scheduler/registry/cdp/plan/llm/quote 指标。
  - 指标示例：`soul_execution_step_latency_ms`, `soul_execution_step_attempts_total`, `soul_plan_rejections_total`, `soul_llm_cache_events_total`, `soul_market_quote_fetch_total`, `soul_manual_takeover_total`。
  - `spawn_metrics_server` 在 CLI 中默认监听 `9090`，可通过 CLI 关闭/修改。
- **State Center**
  - `DispatchEvent::{success,failure}` 记录 actionId/tool/wait/run/pending/slots/error；`PerceiverEvent` 记录 resolve/judge/snapshot/diff；`StateCenterStats` 汇总成功/失败/registry 事件。
  - CLI `scheduler`、`perceiver`、`info` 通过 `format_rfc3339` 输出人类可读时间；`--format json` 可直接喂给 Dashboards。
- **Artifacts / Console**
  - `chat --save-run` 保存 `plans`、`execution`, `state_events`, `artifacts`；`artifacts.rs` 支持 `--extract`（Base64 解码）、`--summary-path`。
  - `console --serve` 使用 `static/console.html` + `/data` JSON API，在本地浏览器查看 run bundle。
- **日志**
  - `init_logging` 支持 `Level::DEBUG`/`Level::INFO`/自定义；可结合 `local.env` 的 `RUST_LOG` 精确控制模块日志。

---

> 后续如果新增模块、开放 gRPC/WebDriver、完善 CDP adapter，请在对应章节添加“关键结构/流程/调试”段落，保持文档与代码同步。
