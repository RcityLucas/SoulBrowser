# 修复方案：提高计划制定/执行的容错能力（参考 BrowserUse 行为）

## 背景
- 当前规则化计划（见 `crates/agent-core/src/planner/rule_based.rs` 输出的 `Navigate -> data.extract-site -> parse -> deliver` 序列）将「银价走势」硬编码到 `https://quote.eastmoney.com/qh/CU0.html`，该页面已返回 404，导致执行阶段始终在 `llm-step-2` 触发 `ObservationGuardrail::Blocked`。
- 执行器 (`crates/soulbrowser-kernel/src/agent/executor.rs`) 检测到阻断后只是在同一计划内重试，没有像 BrowserUse 那样把失败上下文写回 LLM/规划器，缺乏「想法-执行-再判断」闭环。
- 需要将 BrowserUse 的三个核心特性移植到本项目：① 先验证页面是否与目标一致；② 把失败原因实时输入新的计划；③ 提供观察级别的自修复（搜索正确标的、更新解析器等）。

## 目标
1. **计划阶段**：自动插入「目标验证」动作，保证解析/交付步骤只在页面通过校验后执行。
2. **执行阶段**：当 guardrail 报告 404/NotFound 或 URL 不符时，立刻触发 replan，并携带失败摘要、观察片段和 blocker 类型。
3. **知识/策略层**：为金属类查询提供动态 URL 选择与兜底搜索，避免硬编码错误（例如优先 `AG0`，并在 DOM 中确认含“银/white silver”字样）。
4. **回归保障**：覆盖 planner、executor、tools 的单元/集成测试，确保新流程对其它任务无副作用。

## 里程碑 & 工作项

### 1. Planner 注入页面验证步骤
- **触点**：`crates/agent-core/src/planner/rule_based.rs`、`crates/agent-core/src/planner/rule_based.rs` 调用的策略模块、`config/defaults/intent_config.yaml`（确保 intent schema 描述了校验需求）。
- **实现要点**：
  - 在生成导航动作后，插入一个新的 `Custom` 工具（暂定 `data.validate-target`）。参数包含：目标关键词（例如“银价”）、允许的域名单、期望 HTTP 状态。
  - 执行器侧在 `crates/soulbrowser-kernel/src/tools.rs` 中实现 `data.validate-target`：读取最近一次 observation（通过 `ObservationStore` 或传入 `subject_id`），校验 `title/text_sample` 是否含目标关键字，且 `status` 为 200。失败时返回结构化提示并要求 planner 尝试替代 URL。
  - 更新自动修复逻辑（`crates/agent-core/src/planner/rule_based.rs` 中 stage auto-fixes）确保该验证步骤默认位于所有观察前，和当前自动插入的 `data.parse.*`、`Scroll` 行为不冲突。

### 2. Guardrail 触发 → Replan 闭环
- **触点**：`crates/soulbrowser-kernel/src/agent/executor.rs:380-455`、`crates/soulbrowser-kernel/src/replan.rs`、`crates/soulbrowser-kernel/src/block_detect.rs`。
- **实现要点**：
  - 在 `detect_observation_guardrail_violation` 返回 `ObservationGuardrail::Blocked { reason }` 且 `reason` 出自 `Page reports 404/NotFound message` 时，构造 `observation_summary`（title + text_sample）和 `blocker_kind = "page_not_found"` 调用 `augment_request_for_replan`。
  - 将 replan 请求发送到 planner（与现有天气恢复逻辑一致），并把新的 plan 记录进 `FlowExecutionReport`，防止无限重试同一 URL。
  - 当 replan 返回后，把 failure summary 写入 telemetry（方便排查）并附加到计划解释字段（`plans.json` 的 `planner_context.plan_repairs`）。

### 3. 资源/知识层兜底
- **银价 URL 选择**：在 `crates/agent-core/src/planner/rule_based.rs` 或 intent recipe 中，把「银价」意图映射到 `AG0` 或包含 `silver` 的搜索任务。可新增一个 `LookupInstrument` 助手读取配置（例如 `config/defaults/metal_instruments.yaml`），根据请求语义返回正确代码。
- **搜索 fallback**：若验证失败，planner 应追加一条 `Custom` 动作（例如 `search.web` 或 `browser://session/type` + `enter`）搜索“东方财富 白银 走势”。BrowserUse 里的 step 评估/记忆机制提示：确保每次失败都写入 `next_goal` —— 我们可以在 replan 时把“上次 URL 404，请改用搜索入口”加入系统 prompt。
- **解析器兼容**：`crates/soulbrowser-kernel/src/parsers/metal_price.rs` 应能识别新的 DOM 结构（银价页面可能与铜价 DOM 不同）。如需适配，新增 CSS selectors/正则，并在 `crates/soulbrowser-kernel/src/tools.rs:2330+` 的 `data.parse.market_info` 中区分银/铜节点。

### 4. 测试与验证
- **单元测试**：
  - `crates/soulbrowser-kernel/src/block_detect.rs`：新增示例文本，确认只对真正的 404 文案触发。
  - `crates/soulbrowser-kernel/src/replan.rs`：mock `FlowExecutionReport`，验证 `blocker_kind`、`observation_summary` 被注入。
  - `crates/agent-core/src/planner/rule_based.rs`：为金属意图添加 snapshot test，确保 plan 中出现 `data.validate-target`。
- **集成测试**：在 `crates/agent-core/tests/intent_recipes.rs` 或 新增 `tests/browser_guardrail.rs`：模拟请求 -> 第一 plan 打开 404 -> executor 触发 guardrail -> replan -> 第二 plan 使用搜索 + 正确 URL -> `data.parse.market_info` 输出 `metal_price_v1`。
- **手动验证**：在 dev server 上运行一次任务，查看 `plans.json` 与 `executions.json`：应看到 replan 的 `plan_repairs` 备注以及成功的 deliver step。

### 5. 文档与可观测性
- 更新 `docs/agent_guardrails.md`（如无则新建）记录：
  - guardrail 触发路径、replan 触发条件、如何自定义验证关键字。
  - 与 BrowserUse 的差异/相似点，方便后续维护。
- 在 `soulbrowser-output` 的 telemetry 中新增字段 `blocker_kind`（如 `page_not_found`），方便统计规划缺陷。

## 交付物
1. 代码改动（planner/executor/tools/intent配置/tests）。
2. 新/更新文档，描述验证及重规划机制。
3. Telemetry/日志示例，可复现 replan 行为。

## 成功标准
- 面对与本次相同的输入任务，系统会自动：导航 -> 验证失败 -> replan -> 搜索正确页面 -> 解析 + 交付成功。
- Telemetry 显示 replan 原因和最终成功结果，且没有无限重试或人为干预。
