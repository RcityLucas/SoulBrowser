# User-Need Execution Plan

This document describes a concrete plan for ensuring the system can interpret user
requests (e.g., “打开百度搜今天A股行情”), execute all required steps, and deliver
usable results inside task details/output artifacts.

## 1. Intent Capture & Planning Constraints

1. **Structured intent metadata**
   - Extend `AgentRequest.metadata` with fields: `primary_goal`, `target_sites`,
     `required_outputs` (schema), and `preferred_language`.
   - Parse CLI arguments/prompts to fill these fields (simple heuristics + optional LLM).
   - Mirror BrowserUse’s `MessageManager` behavior by wrapping the first user prompt in
     `<initial_user_request>` and tagging follow-ups (`<follow_up_user_request> ...`).
     This keeps the request visible in every step and allows us to diff deltas.
2. **Conversation scaffold similar to BrowserUse**
   - Adopt a canonical system prompt (see `browser_use/agent/system_prompt*.md`) that
     enumerates `<agent_history>`, `<browser_state>`, `<browser_vision>`, `<read_state>`
     and `<file_system>`. SoulBrowser’s planner prompt must output the same channels so
     downstream loops never “forget” user needs.
   - Build a `MessageManager` equivalent that:
     - Maintains `agent_history_items` (with thinking, evaluation, memory, next goal).
     - Injects sensitive data hints and `todo.md` content when relevant.
     - Trims history but always keeps the initial instructions (first item + recent N).
3. **Prompt template updates**
   - Inject structured metadata into the planner prompt and instruct the LLM to cite
     mandatory steps (data acquisition + parsing + output persistence) in `thinking` and
     `next_goal` fields. Template must enforce `action` list JSON mirroring
     BrowserUse’s output contract.
   - ✅ `PromptBuilder` 现已输出 BrowserUse 风格的通道（`<agent_history>`、`<browser_state>`、
     `<browser_vision>`、`<read_state>`、`<file_system>`、`<todo_md>`），并在系统提示里明确要求
     先阅读这些区块再生成 JSON 计划，同时限定 LLM 只能使用既有的自定义工具标识
     （`data.extract-site` / `data.parse.*` / `data.deliver.*` / `agent.note` 等），杜绝凭空造词。
4. **Plan validator**
   - After planner returns steps, run `PlanValidator`:
     - Validate presence/order: `navigate` → `observe` → `act` → `parse` → `deliver`.
     - Verify each user requirement (site, data type, format) has a matching step.
     - Reject plan with actionable error if constraints are missing.
   - Keep a lightweight `todo.md` (BrowserUse uses this file to plan multi-step tasks)
     synced with validator output so user intent stays traceable in artifacts.
   - ✅ `enrich_request_with_intent` 会在 `metadata.todo_snapshot` 中同步 BrowserUse 风格的
     `todo.md` 段落，并在 replan 路径里保持更新，LLM prompt 可直接注入 `<todo_md>` 区块；
     `PlanValidator` 也会拒绝任何未在 allowlist 的自定义动作，并提供可重试的错误提示。

## 2. Observation & Blocker Classification

1. **Observation summary enrichment**
   - Attach `obstruction_type` by scanning `data.text_sample`, `identity`, `url` for
     patterns: `consent_gate`, `captcha`, `unusual_traffic`, `login_wall`, `blank_page`.
   - Include DOM statistics (links, interactive count, scroll containers) similar to
     `AgentMessagePrompt._extract_page_statistics` so LLM can reason about page state.
2. **Runtime signals + watchdogs**
   - When executor hits `observe-fallback`, feed the classification back to the planner
     via `failure_summary` and `latest_observation_summary`.
   - Implement BrowserUse-style watchdogs (`browser_use/browser/watchdogs/*.py`) for
     about:blank, crashes, consent banners, permission dialogs, downloads, popups, and
     screenshot capture. Each watchdog should emit structured events for blocker logs.
3. **Consent/Captcha handlers**
   - Maintain a map of known blockers → remediation steps (accept buttons, alternate
     URLs, manual escalation).
   - Allow registry actions (see §3) to auto-inject helper tools per blocker type.

## 3. Execution Recipes per Intent

1. **Step runner modeled after BrowserUse `Agent.step`**
   - SoulBrowser’s executor loops through: `_prepare_context` (fetch
     `BrowserStateSummary` + screenshot) → `_get_next_action` (LLM call with context) →
     `_execute_actions` (typed tool calls) → `_post_process`.
   - Always capture screenshot + DOM diff before calling the LLM; refresh page-specific
     tool descriptions based on URL (BrowserUse pulls registry prompts via
     `Tools.registry.get_prompt_description`).
2. **Tool & action modeling**
   - Define actions as Pydantic models mirroring `browser_use/tools/service.py` so the
     LLM’s JSON is validated before execution.
   - Build a registry DSL with decorators (`@registry.action(pattern, description)`)
     allowing domain-specific actions to appear only on matching URLs. Inject extra
     “helper” tools (e.g., accept cookie banner) per site/intent.
- ✅ 实现：`config/plugins/registry.json` 的 `helpers[]` 字段现已支持 `pattern`/`description`/`step`（含 `wait`、`timeout_ms`、`tool` 描述），并可通过 `auto_insert: true` 自动把步骤插入到计划里。Prompt 构建器也会把匹配到的 helper 以 “Registry helper actions” 段落写入，便于 LLM 选择。
 - ✅ `news_brief_v1` schema 已加入（`docs/reference/schemas/news_brief_v1.json`），并可通过 `soulbrowser schema lint --schema news_brief_v1 --file output.json` 校验。
 - ✅ 执行器现支持把遗留的 `observe` 自定义动作自动转换为标准 `data.extract-site` 工具，
     并将 `data.deliver.json` 统一映射到结构化输出；针对 GitHub 用户主页新增了
     `data.parse.github-repo` 解析器，会调用公开 API 汇总仓库信息，提高通用意图的可执行性。
3. **Search/Info intent template** (e.g., “搜行情”)
   - Step 1: Navigate to preferred engine (Google/Baidu) with fallback order defined in
     `intent_config.yaml`.
   - Step 2: Observe page → store artifacts.
   - Step 3: Handle blockers (auto-insert consent/captcha helper).
   - Step 4: Perform action (type query, click search).
   - Step 5: Observe results (capture JSON + screenshot).
   - Step 6: Parse results using deterministic parser (extract indices, change %) or LLM
     summarizer when deterministic parser fails.
   - Step 7: Persist outputs (structured JSON + human summary) and update
     `TaskStatusSnapshot.context_snapshot`.
4. **Intent configuration file**
   - YAML describing required steps, fallback sources, output schema. Example fields:
     ```yaml
     intents:
       search_market_info:
         primary_sites: ["google", "baidu"]
         blockers:
           google_consent: accept_google_consent
           google_sorry: switch_to_baidu
         output:
           schema: market_info_v1.json
           include_screenshot: true
     ```

## 4. Data Parsing & Result Delivery

1. **Parser library**
   - Implement `parsers/market_info.rs` (or equivalent) to extract index data/links from
     Baidu/Google. Use DOM snapshots + `ActionResult.extracted_content` to avoid double
     scraping.
   - When parser cannot extract required fields, fall back to LLM summarizer that uses
     `page.observe` content and outputs the schema.
2. **Filesystem + structured outputs**
   - Mimic BrowserUse `FileSystem` semantics: files live under task-scoped roots,
     `read_file`/`write_file` actions update both disk + agent memory. Track attachments
     so CLI/Web console can link to them.
   - Provide a `StructuredOutputAction` analog so LLMs emit schema-valid JSON blobs.
3. **Task outputs**
   - Extend execution manifest to include `structured_output.json` (per schema) and
     reference it in task details.
   - Update CLI/Web console to render `context_snapshot` (contains summary) and provide a
     link to the structured file + screenshot.
4. **Validation**
   - After parsing, validate against schema (e.g., fields `index_name`, `value`,
     `change_pct`). If missing, mark execution as failed with descriptive error.
   - Persist validation errors inside `AgentHistory` (mirroring BrowserUse’s
     `evaluation_previous_goal`/`memory`) so users see why output was rejected.

## 5. Replanning & Fallback Strategies

1. **Self-heal registry additions**
   - Strategies: `auto_retry`, `switch_to_baidu`, `require_manual_captcha`.
   - Each strategy defines the observation patterns that trigger it.
   - Leverage BrowserUse’s consecutive failure counter (see `_force_done_after_failure`)
     to cap retries and to auto-switch to `done(success=false)` when retries are
     exhausted.
2. **Failure summary content**
   - Include both blocker classification and previous outputs so LLM knows why to adjust
     the plan (e.g., “latest observation: Google unusual-traffic page”).
   - Keep `todo.md` synchronized: BrowserUse updates this file + `MessageManager` so the
     loop never loses sight of unfinished subtasks.
3. **Judge / QA pass**
   - Borrow `browser_use/agent/judge.py`: after `done`, evaluate trace vs task to decide
     whether to replan, retry, or accept.
4. **Cache & fallback**
   - Cache successful Baidu plan for repeated “搜行情” requests; reuse when Google route
     fails repeatedly.

## 6. Monitoring & UX surfacing

1. **Task detail panel**
   - Show: last observation summary, parsed result snippet, structured output path,
     screenshot preview, blocker history.
   - Surface `AgentHistory` entries (thinking, evaluation, memory, next goal) the same
     way BrowserUse does in its UI + CLI logs.
2. **Metrics/logs**
   - Counters: `consent_handled_total`, `fallback_to_baidu_total`, `parser_failures`.
   - Alerts when fallback is used more than N times per hour (indicates upstream change).
   - Emit `CreateAgentSession/Task/Step` style events (BrowserUse’s
     `browser_use/agent/cloud_events.py`) for analytics.
3. **Developer visibility**
   - Add tracing spans around planner validation, parser execution, blocker handling to
     aid debugging.
   - Persist raw LLM inputs/outputs for each step (BrowserUse saves via
     `save_conversation` in `message_manager.utils`).

## 7. Implementation Phasing

1. Phase 1: Intent metadata + plan validator + consent auto-step + `MessageManager`
   scaffold (task tags, history persistence).
2. Phase 2: Blocker classification + watchdog/event bus + fallback strategies +
   structured outputs (`StructuredOutputAction`).
3. Phase 3: Parser implementation + schema validation + UI surfacing of
   `AgentHistory`/artifacts + Judge-based QA.
4. Phase 4: Metrics, alerts, telemetry export, doc updates, and registry authoring tools
   for domain-specific actions.

## 9. Current Progress Snapshot (Nov 2025)

- **Intent plumbing** – `AgentRequest` now carries structured intent metadata and
  `<initial_user_request>/<follow_up_user_request>` tags. CLI/API entrypoints propagate
  perception snapshots into `request.metadata.browser_state_snapshot`, so every planner
  call sees the latest URL + DOM context.
- **Plan validation** – The rule-based validator enforces
  `navigate → observe → act → parse → deliver` when structured outputs are requested, and
  deliver steps must emit schema-identified artifacts (JSON + screenshot).
- **Structured output persistence** – `handle_deliver_structured` writes the schema JSON
  and the latest screenshot into the artifact manifest, wiring them through
  `run.manifest.json` for later downloads.
- **Agent history feed** – Each executed step records an `AgentHistoryEntry` (status,
  attempts, observation/obstruction summary, structured-output summary). These entries
  are streamed on the task SSE channel, exposed on `TaskStatusSnapshot`, and fed back
  into replans as `<agent_history>` system blocks.
- **Replan scaffolding** – `augment_request_for_replan` now appends observation + agent
  history context, and runs the blocker-hint logic (e.g. switch to Baidu) before calling
  the planner.
- **Prompt builder updates** – `PromptBuilder` includes `<browser_state>` and `<todo_md>`
  sections whenever metadata/context provides perception snapshots or todo content,
  matching the MessageManager guidance from BrowserUse.
- **Observation enrichment** – Every artifact now carries `dom_statistics` (counts of
  links/headings/paragraphs/interactive elements) and `latest_observation_summary`
  includes those stats so replans immediately see how dense the page is in addition to
  any obstruction classification.
- **News brief recipe** – `summarize_news` intent now generates a deterministic
  navigate → observe → parse(`data.parse.news_brief`) → deliver pipeline that emits
  the `news_brief_v1` schema using the new parser, so planners no longer rely on ad-hoc
  LLM summaries for headline tasks.
- **Watchdog event bus** – Passive watchdogs now emit both annotations and
  `TaskStreamEvent::watchdog` entries with kind/severity, while task snapshots retain the
  most recent events so replans and UIs can react programmatically (e.g. firing
  self-heal strategies).
- **UI surfacing** – Web console’s Live Companion panel shows the streaming watchdog
  events alongside agent history/annotations, so operators can immediately see blocker
  classifications and timestamps without digging into raw logs.
- **Judge / QA pass** – After each successful execution we run `judge::evaluate_plan`
  (schema fulfillment checks), persist the verdict to `TaskStatusSnapshot`/streams, and
  surface it in the UI so missing structured outputs fail fast instead of silently
  returning partial results.
- **Metrics instrumentation** – Prometheus now tracks judge rejections and per-kind
  watchdog counts (`soul_agent_watchdog_events_total`), letting alerts fire whenever
  blockers spike or QA rejects start to climb.
- **Alerts** – Watchdog/self-heal/judge failures emit `TaskAlert` events (streamed via
  SSE and stored in telemetry) so operators can see high-severity issues even when they
  miss the console logs.
- **Telemetry export** – Run manifests include a telemetry section (watchdog events +
  judge verdict + self-heal events) so downstream tooling can correlate structured
  artifacts with blockers/QA/self-heal context without scraping logs.
- **Registry authoring tools** – CLI now ships `registry helper scaffold` for quickly
  generating helper JSON templates (pattern/steps/prompt) alongside the existing
  `helper lint`/`helper add|update|remove` flow, making it easier to maintain
  `config/plugins/registry.json` without hand-editing boilerplate.
- **Registry HTTP scaffold** – `/api/plugins/registry/:plugin/helpers/scaffold` mirrors the
  CLI functionality so the Web Console (and other tooling) can request helper templates
  dynamically when authoring/previewing registry entries.
- **Self-heal fallback coverage** – `blank_page` blockers leverage the existing
  `auto_retry` strategy by default (intent configs + obstruction detection wired through
  DOM statistics), so stuck observations trigger retries or replans without manual
  intervention.
- **Tool/test ergonomics** – `BrowserToolManager::with_executor` + `MockExecutor` allow
  the tool tests to run without launching Chromium, keeping CI fast while the real
  executor remains unchanged.

## 8. Borrowed Practices from BrowserUse

BrowserUse 的 Agent 架构提供了一套经过验证的“持续感知 + LLM 步进 + 丰富工具”模式。为让 SoulBrowser 对用户需求更灵活地响应，我们将其关键做法分阶段吸收：

### 8.1 任务摄取 & 会话状态 (browser_use/agent/message_manager/service.py)
- `Agent(task=...)` 将用户请求包装进 `MessageManager`，并在每个 step 中附带过去的思考、记忆、下一目标。
- `MessageManager.add_new_task` 会在收到新需求时追加 `<follow_up_user_request>`，并把 `todo.md` / `file_system` 摘要写入 `<agent_state>`，确保 LLM 始终看见完整上下文。
- `SystemPrompt` (system_prompt*.md) 详细列出 `<agent_history>`、`<browser_state>`、`<browser_vision>`、`<read_state>` 与输出 JSON 结构，我们也需要同样严格的模板来避免幻觉。

### 8.2 Step Runner & 浏览器上下文 (browser_use/agent/service.py)
- 每一步严格执行 `_prepare_context → _get_next_action → _execute_actions → _post_process → _finalize`，中间多次调用 `_check_stop_or_pause` 防止僵死。
- `_prepare_context` 总是抓取截图、DOM 摘要、事件流以及下载列表 (`_check_and_update_downloads`)，并动态注入当前 URL 的可用动作。
- `_get_next_action` 使用 `MessageManager` 生成消息列表，超时会把输入写入日志（`observe('_llm_call_timed_out_with_input')`）。
- `_execute_actions` 顺序执行 LLM 输出的动作列表，失败会写入 `ActionResult(error=...)` 并递增 `consecutive_failures`。

### 8.3 工具注册 & 站点定制 (browser_use/tools/service.py + tools/registry/service.py)
- 所有动作都是 Pydantic `ActionModel`，LLM 输出 JSON 先反序列化再执行，确保参数合法（例如 `navigate`, `click`, `extract`, `write_file`, `done`).
- `Registry.action(pattern=...)` 允许按 URL/域名注入特定动作；浏览器状态里也会展示这些动作的描述，帮助 LLM 选择。
- 工具执行依赖 Playwright CDP 事件 (`NavigateToUrlEvent`, `ClickElementEvent`, ...)，并将错误转换成可读 `ActionResult`。

### 8.4 数据面 & 结果提交
- `FileSystem` 负责 `read_file`/`write_file`/`list_files`，并把引用写进 `AgentHistory`，便于后续步骤读取。
- `StructuredOutputAction`/`DoneAction` 让 LLM 在完成时返回 schema 校验后的 JSON + `success` flag，CLI 会展示附件路径。
- `ActionResult` 同时携带 `extracted_content` 和 `long_term_memory`，下一步在 `<agent_history>` 中回放，帮助 LLM判断进度。

### 8.5 观测、回放与 QA
- `browser_use/browser/watchdogs/*` 对空白页、崩溃、权限、下载、截图等事件做实时监控，必要时向 LLM 提示异常。
- 每步结束都会记录 `AgentHistory` 与 `CreateAgentStepEvent`（包含 URL、操作、截图路径、耗时），并可选触发 `judge.py` 做二次评估。
- `ProductTelemetry`、`EventBus` 用来集中汇报 session/task/step 生命周期事件，方便 SaaS 端可观察性。

### 8.6 对 SoulBrowser 的落地动作
1. 复制 `MessageManager` 样式：统一的系统提示 + history/traces + todo.md 注入。
2. 复用 Step Runner 模型：Sense→Think→Act→Finalise，并在每步强制抓屏/DOM diff。
3. 构建强类型工具矩阵 + 站点特定 action 注入，使 LLM 输出天然符合执行层接口。
4. 保留 `ActionResult`/`AgentHistory` 文档化，确保文末 QA/judge 有完整语境。
5. watch dog + telemetry + judge 组合，为 replanning/fallback 提供事实依据。

以上阶段会在现有计划的基础上展开，确保我们逐步具备 BrowserUse 式的灵活性：实时感知浏览器状态、针对不同站点提供定制工具，以及在需要时切换代理/持久化会话、同步文件。
