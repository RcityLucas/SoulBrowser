# SoulBrowser（中文概览）

> 智能浏览器自动化框架：多模态感知、策略守护、可编排 CLI。

本文件提供 SoulBrowser 工作区的中文速览，帮助团队成员快速了解仓库结构、主要组件、常用命令与开发流程。若需更详细/完整的说明，请参考英文版 `README.md` 以及 `docs/` 目录下的模块文档。

## 目录
1. [核心特性](#1-核心特性)
2. [目录导航](#2-目录导航)
3. [常用命令](#3-常用命令)
4. [配置流程与环境变量](#4-配置流程)
5. [架构速览](#5-架构速览)
6. [观测与工件](#6-观测--工件)
7. [开发流程](#7-开发流程)
8. [现状与路线图](#8-现状与路线图)
9. [许可证](#9-许可证)
10. [延伸阅读](#10-延伸阅读)

## 1. 核心特性
- **统一 CLI**：`soulbrowser` 支持 Serve/Gateway、感知、Agent Chat、调度/策略观测、工件导出等所有子系统。
- **可复用内核**：`soulbrowser-kernel` 负责 Chrome/CDP 启动、AppContext 构建、调度/策略/插件/内存/存储等基础设施，并暴露 Serve/Gateway API。
- **多模态感知**：结构（DOM/AX）、视觉（截图/视觉指标/OCR）、语义（语言/摘要/关键词）感知模块可组合输出 `MultiModalPerception`，供 CLI 与 Console 使用。
- **动作 + 调度链路**：`action-*` crate 提供六大原语 + 选择器愈合 + Gate 验证，`scheduler` 结合 `registry`、`state-center`、`policy-center` 完成安全的工具调度。
- **可观测性**：Prometheus Metrics、State Center 快照、Event Store + Timeline 导出、内置 `info`/`scheduler`/`perceiver`/`policy` 命令。
- **治理与扩展**：权限/策略中心、隐私红线、L7 Gateway/Plugin Runtime、Memory Center、Timeline、Permissions Broker、Extensions Bridge 等支撑企业级治理。

## 2. 目录导航
| 路径 | 说明 |
| --- | --- |
| `src/cli/` | CLI 命令实现（serve/perceive/chat/analyze/artifacts/console/...）。|
| `src/bin/` | 辅助二进制（如 `parser_scaffold.rs`）。|
| `crates/` | 所有内部 crate（内核、感知、动作、调度、策略、插件、网关等）。|
| `config/` | 配置示例、策略/权限/插件/Planner 资源、`local.env`。|
| `static/` | 控制台 Web Shell (`console.html`)。|
| `soulbrowser-output/` | 默认输出目录（租户存储、截图、Run Bundle、State Center 快照等，已忽略提交）。|
| `third_party/` | 第三方资源（如有）。|
| 根目录 `Cargo.toml`/`Cargo.lock`/`build.rs` | Workspace 配置、依赖锁定、编译时 metadata（构建时间、Git hash/branch）。|

### 2.1 关键 crate
- **Kernel & Runtime**：`crates/soulbrowser-kernel`（`kernel.rs`、`runtime.rs`、`app_context.rs`、`perception_service.rs`、`metrics.rs`、`gateway/`、`server/` 等）。
- **动作栈**：`action-primitives`、`action-locator`、`action-gate`、`action-flow`、`soulbrowser-actions`。
- **感知栈**：`perceiver-structural`、`perceiver-visual`、`perceiver-semantic`、`perceiver-hub`、`network-tap-light`。
- **控制面**：`registry`、`scheduler`、`state-center`、`policy-center`、`event-store`、`snapshot-store`、`memory-center`、`event-bus`。
- **集成/治理**：`cdp-adapter`、`permissions-broker`、`extensions-bridge`、`stealth`、`l6-observe`、`l6-privacy`、`l6-timeline`、`l7-adapter`、`l7-plugin`、`integration-soulbase`。
- **Agent & LLM**：`agent-core`（计划/模型/执行转换）、`soulbrowser-kernel::agent`、`chat_support`。

## 3. 常用命令
```bash
# 构建全部 crate
cargo build --workspace

# 启动 Serve Console（默认 127.0.0.1:8787）
cargo run --bin soulbrowser -- serve --surface console --port 8787 --auth-token devtoken

# 多模态感知
ecargo run --bin soulbrowser -- perceive --url https://example.com --all \
    --output ./soulbrowser-output/perception/example.json \
    --screenshot ./soulbrowser-output/perception/example.png

# L8 Agent 规划 + 执行
ecargo run --bin soulbrowser -- chat --prompt "订一张往返机票" --execute \
    --save-run ./soulbrowser-output/runs/flight.json --artifacts-path ./soulbrowser-output/runs/artifacts.json

# 调度/感知/策略/TL 观测
ecargo run --bin soulbrowser -- scheduler --status failure
cargo run --bin soulbrowser -- perceiver --kind resolve --format table
cargo run --bin soulbrowser -- policy show --json
cargo run --bin soulbrowser -- timeline --view records --action-id <ACTION>

# 动态工具注册/管理
cargo run --bin soulbrowser -- tools register --file ./config/tools/sample.json
cargo run --bin soulbrowser -- tools list

# Telemetry 实时查看
cargo run --bin soulbrowser -- telemetry tail

# 离线控制台
ecargo run --bin soulbrowser -- console --input ./soulbrowser-output/runs/flight.json --serve
```

## 4. 配置流程
1. **复制示例**：`config/config.yaml.example → config/config.yaml`，设置 `default_browser`、`output_dir`、`policy_paths`、`strict_authorization`、`serve_surface` 等。
2. **环境变量**：在 `config/local.env` 写入 `SOUL_CONSOLE_TOKEN`、LLM API Key、`SOULBROWSER_CHROME` 等，CLI 在启动前自动加载。
3. **策略/权限/插件/工具**：根据需要维护 `config/policies`、`config/permissions`、`config/plugins`、`config/planner`，以及 `config/tools/*.json` 中的自定义工具（可通过 `soulbrowser tools register/remove` 命令管理）；Telemetry Sink 配置持久化在 `config/telemetry.json`，`soulbrowser telemetry ...` 命令会自动更新并在启动时加载。
4. **输出目录**：默认 `soulbrowser-output`，可通过配置或 CLI 参数覆盖。

### 4.1 常用环境变量
| 变量 | 作用 |
| --- | --- |
| `SOULBROWSER_CHROME` / `SOULBROWSER_USE_REAL_CHROME` | 指定或强制使用真实 Chrome/Chromium 路径。|
| `SOULBROWSER_WS_URL` | 连接已有的 DevTools WebSocket，而非本地启动 Chrome。|
| `SOULBROWSER_LLM_CACHE_DIR` | Planner LLM 缓存目录。|
| `SOULBROWSER_DISABLE_PERCEPTION_POOL` | 禁用共享感知会话池。|
| `SOUL_STRICT_AUTHZ` | 强制启用严格授权。|
| `SOUL_SERVE_SURFACE` | `serve` 默认 Surface（console/gateway）。|
| `SOUL_CONSOLE_TOKEN` / `SOUL_SERVE_TOKEN` | Serve Surface 访问 Token。|
| `SOUL_POLICY_PATH` | 策略快照路径（否则按 `policy_paths` 搜索）。|
| `SOUL_CHAT_CONTEXT_LIMIT` / `SOUL_CHAT_CONTEXT_WAIT_MS` | Chat 上下文并发&等待时间。|

## 5. 架构速览
```
CLI → runtime 引导（env/config/logs）
     → dispatch 到 kernel serve/gateway
          → AppContext（Storage/Auth/Tools/Registry/Scheduler/StateCenter/Policy/Plugin）
          → Perception Service → Perceiver Hub
          → Event Store + Timeline + Memory Center
          → L7 Adapter / Serve Console / Gateway HTTP
```

- **调度链路**：`scheduler` 通过 `registry` 获取 `ExecRoute`，结合 `policy-center`、`state-center`、`permissions-broker` 控制工具执行与重试。
- **感知链路**：`perception_service` 协调结构/视觉/语义感知，支持截图、日志写入、共享会话池，输出 `MultiModalPerception`。
- **治理链路**：`policy-center` 支持 `RuntimeOverride`，`l7-adapter` + `l7-plugin` 提供外部 API 与插件运行时，`l6-privacy`/`permissions-broker` 负责合规。

## 6. 观测 & 工件
- **Metrics**：启动 `soulbrowser` 时默认打开 `http://localhost:<metrics_port>/metrics`（默认 9090）。指标覆盖 Scheduler/Registry/CDP Adapter/LLM Cache/方案验证等。
- **State Center**：保留最近的调度/感知事件，可通过 `soulbrowser scheduler|perceiver|info` 查看，快照存储在 `soulbrowser-output/state-center/`。
- **Timeline**：`soulbrowser timeline` 会从存储拉取事件，写出 Records/Timeline/Replay 文本或文件。
- **Run Bundle**：`chat --save-run` 输出 `plans`、`execution`、`state_events`、`artifacts`，可被 `console`/`artifacts` 命令复用。
- **Artifacts/GIF**：`soulbrowser artifacts --gif timeline.gif --gif-frame-delay 350 --gif-max-frames 120` 可将筛选后的截图工件组合成 BrowserUse 风格的 GIF 时间线，便于快速回放。
- **Telemetry**：设置 `SOULBROWSER_TELEMETRY_STDOUT=1` 即可在执行期间打印 JSON 步骤事件，便于串接外部 Webhook/PostHog（可通过 `soulbrowser_kernel::telemetry::register_sink` 扩展）。
- `soulbrowser telemetry tail` / `telemetry webhook --url ...` / `telemetry posthog --api-key ...` 可实时查看或转发事件；若 LLM provider 返回 usage，事件里的 `llm_input_tokens`/`llm_output_tokens` 会自动填充。

## 7. 开发流程
1. **格式化与静态检查**
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   ```
2. **测试**
   ```bash
   cargo test --workspace
   ```
3. **专项调试**：针对某个 crate 执行 `cargo test -p <crate>` 或 `cargo run -p <crate> --example ...`。
4. **端到端验证**：`cargo run --bin soulbrowser -- serve ...`、`-- chat ...`、`-- perceive ...` 等。

## 8. 现状与路线图
- CDP Adapter、Network Tap、Permissions Broker、Extensions Bridge 仍为脚手架，预计按里程碑逐步补完。
- 旧 CLI 命令 `start/run/record/replay/export/demo` 暂时直接报错提醒使用 `serve/gateway/chat` 等新流程。
- Gateway 目前仅提供 HTTP Surface；gRPC/WebDriver Listener CLI 选项已预留但未实现。
- LLM Planner 支持规则/LLM/Mock，多模型与 API 需通过 CLI 或环境变量配置。
- Plugin Registry/Policy Override/Privacy Filter 依赖 `config/plugins`、`config/policies`、`config/policies` 下的组织定制内容。

## 9. 许可证
项目采用 MIT / Apache-2.0 双许可（`Cargo.toml` 中声明）。

## 10. 延伸阅读
- 英文主文档：`README.md`
- 模块深度解析：`docs/module_deep_dive.md`
- 模块概览：`docs/README.md`
- CLI 命令实现：`src/cli/`
- Kernel 核心：`crates/soulbrowser-kernel/`
