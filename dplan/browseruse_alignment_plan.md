# BrowserUse Parity Roadmap

## Goal
Mirror the BrowserUse reference implementation (`E:/projects/browseruse/browser-use/browser_use`) so SoulBrowser can run the same "对话→搜索→点击权威站→验证→解析→交付" loop even when the LLM falls back to rule plans.

## Workstream A – Deterministic Search & Action Pipeline
1. ☐ **Stage graph + StageAuditor overhaul** – In `config/planner/stage_graph.yaml` / `config/defaults/stage_graph.yaml` declare the exact Navigate→Act→Observe→Validate→Parse→Deliver chains for informational intents, then update `StageAuditor` (`crates/soulbrowser-kernel/src/agent/mod.rs`) to *always* insert `browser.search`, `auto act`, `data.extract-site`, and `data.validate-target` when the plan is missing those stages.
2. ☐ **BrowserUse-style search schema** – Model `browser.search`, click, and text-input payloads after BrowserUse’s `browser_use/tools/views.py` (`SearchAction`, `ClickElementAction`, etc.). Update `crates/soulbrowser-kernel/src/tool_registry.rs` + `config/tool_registry/*.json` so LLMs see the same field hints BrowserUse exposes.
3. ☐ **AutoAct strategy with builtin click** – Extend `AutoActStrategy` (`crates/soulbrowser-kernel/src/agent/strategies/act.rs`) to detect Baidu/Google/Bing result URLs, automatically type the query, submit, and click the first result *with domain validation* (similar to BrowserUse’s deterministic skills). Remove the current “wait-for-condition URL matches search page” loops that stall execution.
4. ☐ **Observation + validation alignment** – Ensure `ExtractSiteObserveStrategy` and `TargetGuardrailStrategy` always pair: observation URLs must match the clicked authority site, and `data.validate-target` should inherit keywords/domains from intent metadata. Reference BrowserUse’s guardrail logic in `browser_use/agent/views.py` (thinking/evaluation fields) when designing metadata.

## Workstream B – BrowserUse Messaging & Memory
1. ☐ **MessageManager equivalent** – Implement a lightweight message manager in SoulBrowser (e.g., `crates/soulbrowser-kernel/src/chat_support.rs`) inspired by `browser_use/agent/message_manager/service.py`. It should maintain `<initial_user_request>`, `<follow_up_user_request>`, `<read_state_x>` snippets, and cap history length.
2. ☐ **Thinking/Evaluation/NextGoal surfaces** – Wire the auto-generated `agent_state` blobs (from Stage strategies) into Serve/CLI timelines, mirroring BrowserUse’s Evaluate/NextGoal/Memory panels. Reference `browser_use/agent/views.py::AgentOutput` for naming.
3. ☐ **Judge + rerun parity** – Align our judge/replan notes with BrowserUse’s `ActionResult/judgement` flow so the Serve UI can show verdicts and rerun summaries consistently.

## Workstream C – Guardrail-driven Search Seeding & Tests
1. ☐ **Guardrail keywords → search terms** – Extend `StageContext` to merge `derive_guardrail_keywords` into `search_terms` (in progress) and log them for telemetry so replans always reuse authority-specific queries.
2. ☐ **Integration tests** – Add end-to-end tests proving a plain informational prompt (“我想看下现在办理最多的案件…”) yields a plan that contains `browser.search`, auto click, `data.extract-site`, `data.validate-target`, parse, and deliver steps even when the LLM planner is rate-limited (similar to BrowserUse’s deterministic runs).
3. ☐ **Documentation** – Update `docs/agent_guardrails.md` and add a new `docs/browseruse_parity.md` describing how search terms/domains are configured, how the new stage strategies work, and how to troubleshoot guardrail-driven replans.

## Workstream D – Serve/CLI UX & Demo Assets
1. ☐ **UI telemetry** – Mirror BrowserUse’s animated panels by mapping `agent_state` and `MessageManagerState` into Serve overlays and CLI JSON (similar to BrowserUse’s console). Files: `crates/soulbrowser-kernel/src/server/router/chat.rs`, `src/cli/chat.rs`.
2. ☐ **Demo scripts** – Add runnable scripts (similar to `browser_use/examples`) that launch typical informational tasks, collect GIFs/logs, and verify the new pipeline.
3. ☐ **Skill/Tool docs** – Document the new tool schemas (`config/tool_registry/guardrails.json`, etc.) so contributors know how to add BrowserUse-compatible actions.

## Immediate Next Steps
1. Implement the Stage/strategy changes in Workstream A items 1–3 (directly referencing the BrowserUse codepaths above).
2. Build the MessageManager shim and UI surfaces from Workstream B item 1 to unblock BrowserUse-style Evaluate/NextGoal signals.
3. Once the deterministic pipeline is in place, write integration tests + docs (Workstream C) before polishing Serve UI/demos (Workstream D).
