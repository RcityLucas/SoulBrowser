# Agent Guardrails & Replan Flow

This document summarizes the new validation and guardrail workflow introduced for
market-intent tasks.

## Target validation与评估阶段

* The rule-based planner now inserts a `data.validate-target` step immediately
  after each observation (`data.extract-site` or `market.quote.fetch`).
* The planner derives validation keywords and allowed domains from intent
  metadata (`validation_keywords`, `allowed_domains`) and the detected metal
  symbol/contract. These hints can be configured per intent in
  `config/defaults/intent_config.yaml`.
* The new tool checks the latest observation snapshot for:
  - Required keywords inside the title/text sample
  - Allowed domains (with wildcard support for subdomains)
  - Expected HTTP status (defaults to `200`)
  - Access blocks (delegates to `block_detect.rs`)
* Tool output emits structured metadata so downstream stages and telemetry know
  which domain/status/keywords were verified.
* 每次观察完成后都会自动插入 `agent.evaluate` 步骤，记录人类可读的“评估”说明（例如“页面仍为 404”，或“已关闭弹窗，继续滚动”）。这些评估既保存在执行轨迹中，也作为 guardrail 的额外上下文在失败时反馈给 replanner。

## Guardrail → Replan loop

* `execute_plan` now attaches guardrail context to `StepExecutionReport` entries
  (`observation_summary`, `blocker_kind`). These details are propagated to
  telemetry (`task_status` history, `plans.json`, `executions.json`).
* When an observation guardrail fires (URL mismatch, Baidu weather fallback, or
  access blocks such as 404/403) **or** a domain tool such as `market.quote.fetch`
  fails repeatedly, we capture the latest title/snippet and emit a
  machine-readable `blocker_kind`（例如 `page_not_found`、`quote_fetch_failed`）。 This data is fed into
  `augment_request_for_replan` so replans receive:
  - A failure summary describing the offending step
  - The observation snippet that triggered the guardrail
  - Blocker-specific system guidance (e.g., "last page returned 404, search via
    Baidu for a fresh quote page and validate before parsing")
* CLI replanning now goes through `soulbrowser_kernel::replan::augment_request_for_replan`
  and `ChatRunner::replan`, ensuring the new metadata is surfaced in
  `plan_repairs` and avoiding duplicate URL retries.

## Guardrail 搜索刷新与兜底

* `StageContext` 暴露 `guardrail_queries()`，会基于 guardrail 关键词、域名、`site:` 限定以及
  常见 fallback 词（行情/报价/走势等）生成一组从“严格”到“宽松”的查询。
* `SearchNavigateStrategy` 使用第一条查询触发初始 `browser.search`，而
  `browser.search.click-result` 的 metadata 中会存储完整的查询队列，用于 Guardrail 刷新。
* 当 AutoAct 候选耗尽并返回 `[auto_act_candidates_exhausted]` 时，执行器会按照该队列依次触发
  `browser.search`：先尝试带 `site:` 的权威域，再退化到域名关键字，最后退到“目标 + 行情/报价”
  这类泛搜。超过队列后还会自动附加原始 goal 与“最新”提示，然后才触发 replanning。
* 就算 AutoAct 没有显式输出 `[auto_act_candidates_exhausted]`，只要 `browser.search.click-result`
  在 60 秒内迟迟跳不到 Guardrail 域（scheduler 报 `timed out`），同样会触发刷新逻辑，避免卡死在单个
  SERP。
* 这样 SERP 没有权威跳链时，系统也能像 BrowserUse 那样继续刷新/降级，最大化 guardrail 覆盖。

## Blocker guidance & search fallback

* `AgentIntentMetadata` accepts `validation_keywords` and `allowed_domains` so
  intent recipes can describe the expected page semantics. These values are used
  both for validation and for generating fallback hints.
* When the blocker is `page_not_found`, the planner receives a predefined Baidu
  search URL for the current goal (e.g. `https://www.baidu.com/s?wd=<goal>%20行情`)
  to encourage dynamic source discovery.
* `plan.meta.vendor_context.plan_repairs` now records the failure summary for
  replanned sessions, making it visible in `plans.json` as `last_failure_summary`.

## Telemetry visibility

* `flow_execution_report_payload` and CLI `execution.json` include
  `observation_summary` and `blocker_kind` for every step（导航/评估/交付都携带自然语言说明），making guardrail
  failures and成功路径 equally easy to inspect post-run.
* `TaskStatus` history entries store the same information, so the console and
  future judge heuristics can reason about repeated guardrail trips.
* Each plan/execution step now preserves the planner-provided `thinking`,
  `evaluation`, `memory`, and `next_goal` snippets (under `agent_state`). These
  fields show up in `plans.json`, `executions.json`, and the task history so UI
  timelines can render BrowserUse-style Evaluate cards instead of opaque note
  entries.

## Tool registry configuration

* A descriptor for `data.validate-target` now ships in
  `config/tool_registry/guardrails.json`. The serve/CLI bootstrap loads every
  JSON file inside `config/tool_registry` and merges them with the built-in
  descriptors.
* You can tweak the descriptor (keywords, description, prompt text, etc.) or add
  new guarded tools by dropping additional JSON files into that directory.
* Tool registries are only read at startup today. After editing the files,
  restart the `soulbrowser serve …` process (or rerun the CLI command) so the new
  definitions take effect. Hot reload can be added later, but this one-step
  restart keeps the workflow predictable for now.
