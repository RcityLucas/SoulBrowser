# BrowserUse Parity Follow-up Plan

## Goal
Bring SoulBrowser's real-world behavior (execution + UX) in line with BrowserUse so that a typical informational task completes the deterministic "search → auto act → observe → validate → parse → deliver" loop without manual intervention.

## Workstream 1 – Deterministic Stage Graph
1. **Planner override**
   - For informational intents, StageAuditor should output the exact BrowserUse chain (Navigate → Auto → Observe → Validate → Parse → Deliver) even if the LLM produced extra steps. Missing or out-of-order stages must be removed/rewritten, not merely appended.
   - Record stage-level overlays/notes (guardrail, auto-act, observe, deliver) so Serve can “tell the story” of each plan the same way BrowserUse’s telemetry timeline does.
2. **Route-aware observation**
   - Stage strategies that produce `data.extract-site` must first navigate to / focus the authority tab and push their own navigation step so the execution route always matches the expected URL metadata.
   - Tie `EXPECTED_URL` metadata to Validate/Deliver so reporting clearly states which URL was observed and verified.

## Workstream 2 – Browser Automation Parity
1. **`browser.search` parity**
   - Normalize `site:` hints to hostnames, accept per-engine selectors, and treat “empty result” pages as recoverable fallbacks. Log engine retries via telemetry.
2. **AutoAct execution**
   - Mirror BrowserUse’s SearchAction: retype query, submit, click preferred domain, and rely on guardrail-domain waits (not hard-coded URLs). Remove fragile waits on Baidu’s input box.
3. **New-tab routing**
   - When clicks spawn new tabs/windows, automatically switch the ExecRoute to the new page so all subsequent waits/observations operate on the authority tab instead of timing out on the search page.

## Workstream 3 – Reporting & Demo Experience
1. **Serve UI parity**
   - Consume `message_state` and telemetry streams to render Evaluate / NextGoal / Guardrail / AutoAct badges like BrowserUse’s console. Highlight deterministic stage transitions so users can audit each stage quickly.
2. **CLI/Serve demo scripts**
   - Provide a recorded walkthrough (log + GIF) using the deterministic flow, storing artifacts under `docs/demos/`.
   - Document how to inspect telemetry counters (`soul_guardrail_keyword_seeds_total`, `soul_auto_act_search_engine_total`).
3. **Headless smoke tests**
   - Add a scripted CLI run (non-network mock) verifying the plan includes search → auto act → observe → validate → parse → deliver even when the LLM planner is unavailable.

## Immediate Next Steps
1. Update StageAuditor to “own” informational plans: enforce the BrowserUse stage sequence, rewrite/trim extra steps, and emit guardrail/auto-act overlays.
2. Implement ExecRoute switching for new tabs so waits/observations follow the page opened by AutoAct clicks.
3. Refresh Serve/CLI demos once telemetry reflects the deterministic pipeline (guardrail keywords, auto-act engine, validate success).
