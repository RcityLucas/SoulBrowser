# Plan: Achieve BrowserUse-Level Capability

## Background
- `browser-use` relies on an LLM-driven loop (think → act → evaluate → next goal) with a MessageManager, judge, and telemetry for every step.
- `soulbrowser` still falls back to a rule-based `navigate -> observe -> parse -> deliver` template. StageAuditor patches missing stages but cannot replicate BrowserUse Cloud’s adaptive retries, popup handling, or per-step critique.
- Missing behaviors observed in customer logs:
  - Plans still emit unsupported `plugin.*` tools or single-shot `market.quote.fetch` calls that hit a 404 and stop.
  - No search fallback or popup handling because LLM replanning is bypassed after rate limits.
  - Telemetry lacks per-step reasoning, so UI cannot render BrowserUse-style Evaluate/Next Goal timeline.

## Objectives
1. Make LLM planning the primary path; rule-based planner only acts as a bootstrap when LLM is unavailable.
2. Adopt BrowserUse’s action → evaluation loop, including mandatory `agent.evaluate` steps and natural-language summaries.
3. Allow the agent to discover new pages dynamically (search, close popups, interact) instead of relying on static URLs.
4. Ensure guardrail triggers (404, quote fetch failure, captcha) feed structured context back to the LLM so replans contain corrective actions.
5. Provide telemetry comparable to BrowserUse Cloud: each action logs rationale, evaluation, memory, next goal.

## Status Snapshot (2024-XX-XX)
- [x] Planner & prompt overhaul: `PromptBuilder` now enforces BrowserUse thinking/evaluation/memory/next_goal metadata, and every entry point (CLI/Serve API) defaults to the LLM planner, only falling back to the rule planner when the provider fails (`crates/soulbrowser-kernel/src/llm/prompt.rs`, `crates/soulbrowser-kernel/src/server/router/chat.rs`, `src/cli/chat.rs`).
- [x] Toolchain alignment: plugin aliases normalize into canonical tools, BrowserUse-style helpers (`browser.search`, popup controls) exist in the allowlist/registry, and tests cover alias rewrites (`config/planner/custom_tool_allowlist.json`, `crates/soulbrowser-kernel/src/agent/mod.rs`, `crates/soulbrowser-kernel/src/tool_registry.rs`).
- [x] Execution loop & guardrails: agent.evaluate enforcement, structured observation summaries, blocker hints, and judge/memory hooks are wired through the executor and replan helpers so replans always receive BrowserUse-grade context (`crates/soulbrowser-kernel/src/agent/executor.rs`, `crates/soulbrowser-kernel/src/replan.rs`, `docs/agent_guardrails.md`).
- [x] Telemetry parity: step metadata (thinking/evaluation/memory/next_goal) plus plan repair logging are persisted to task history (`crates/soulbrowser-kernel/src/agent/mod.rs`, `crates/soulbrowser-kernel/src/task_status.rs`).
- [x] Data-source resilience: `market.quote.fetch` rotates DOM/API sources, marks unhealthy feeds, and emits `quote_fetch_failed` blockers so Search-first replans trigger automatically (`crates/agent-core/src/planner/quote_sources.rs`, `crates/soulbrowser-kernel/src/tools.rs`).
- [ ] Nice-to-have artifacts: GIF/screenshot timeline export hooks from BrowserUse Cloud remain optional backlog; keep capturing individual screenshots but skip auto-GIF generation until required.

## Work Breakdown

### 1. Planner & Prompt Overhaul
- **Integrate BrowserUse prompt**: port `browser_use/agent/prompts.py` thinking/evaluation format into `crates/soulbrowser-kernel/src/llm/prompt.rs`, including sections for thinking, evaluation, memory, and next goal.
- **LLM-first strategy**: update `PlannerStrategy` so `PlannerStrategy::Llm` is default; only fall back to `RuleBasedPlanner` if LLM returns an error. Remove StageAuditor shim that overwrites LLM plans unless a stage is truly missing.
- **Replan data model**: extend `augment_request_for_replan` to include `observation_summary`, `evaluation text`, `quote_fetch_failed` hints, and prior tool history. Mirror BrowserUse’s `AgentState` by storing thinking/evaluation in metadata.

### 2. Toolchain Alignment
- **Canonicalize plugin aliases**: already started for `plugin.extract-site`; finish coverage for other plugin names (e.g., `plugin.auto-scroll`, `plugin.data-parse.generic`). Add tests ensuring every plugin alias maps to a real tool.
- **Search & popup tools**: implement general-purpose tools (search page, close modal, send ESC) similar to BrowserUse skills. Expose them inside `config/planner/custom_tool_allowlist.json` so LLM can call them.
- **Custom tool registry**: create a Rust equivalent of BrowserUse’s `Tools` registry so developers can register new actions with JSON schemas. Planner should read this registry to advertise available tools to the LLM.

### 3. Execution Loop Enhancements
- **Evaluate stage enforcement**: ensure every DOM interaction is followed by `agent.evaluate`, and store human-readable summaries in `observation_summary`. These summaries should be persisted to `soulbrowser-output/.../executions.json`.
- **Memory & judge hooks**: add lightweight memory (last observation, obstacles) and an optional judge similar to BrowserUse’s `JudgeResult`. This allows guardrail detections to be validated before final done/fail status.
- **Guardrail-to-replan mapping**: extend guardrail detection with new blocker kinds (`quote_fetch_failed`, `search_no_results`, `popup_unclosed`). Map each blocker to a concrete replan hint (e.g., “switch to search flow”, “send ESC before continuing”).

### 4. Telemetry & UI Improvements
- **Rich step payloads**: modify telemetry (task status registry, `plans.json`, `executions.json`) to include `thinking`, `evaluation`, `next_goal`. Each step should mirror BrowserUse’s timeline (search, click, evaluate, send_keys, evaluate, ...).
- **Plan repair logging**: record when LLM replans or StageAuditor inserts placeholders, so console output matches BrowserUse’s “planner critiques”.
- **Artifacts & gifs (optional)**: BrowserUse can generate GIF summaries; consider exposing a similar hook by saving screenshot sequences and optionally rendering them offline.

### 5. Data Source Resilience
- **Quote source rotation**: keep new Sina sources but add heuristics to detect stale DOM and automatically switch to fallback (e.g., API vs DOM). When a source returns 404 or empty data, log `quote_fetch_failed` and trigger LLM suggestions (search for “银价 新浪 财经”).
- **Search-first workflow**: update market intent recipes so the first steps are search/interact/evaluate, not direct EastMoney navigation. Use LLM reasoning to decide when the goal has been satisfied.

## Deliverables
1. Updated planner prompt + LLM-first execution path.
2. Canonical plugin-to-native tool mapping with tests.
3. New tools for search/popup handling and a registry for custom actions.
4. Enhanced telemetry schema documenting thinking/evaluation per step.
5. Guardrail + replan integration docs in `docs/agent_guardrails.md`, describing new blockers and recovery flows.

## Dependencies & Risks
- Requires stable LLM access (e.g., ChatBrowserUse). Need fallback providers (Claude, GPT-4) if rate limit errors persist.
- Large telemetry changes will affect downstream consumers (CLI/UI); coordinate schema updates.
- BrowserUse tooling grows quickly; consider periodic diff/review to stay aligned with upstream features.

## Next Steps
1. Prototype the new prompt + LLM-first runner using a sample task (银价查询) until BrowserUse-like traces appear.
2. Ship tool canonicalization patches so plugin steps no longer break execution.
3. Gradually replace rule-based planner with LLM planner, keeping rule-based as last-resort fallback.
