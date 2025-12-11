# Adaptive Plan: Perceive First, Act Later

## Goal
Ensure every LLM-generated execution plan starts by grounding itself in live page context *and* reuses that context throughout subsequent actions.

## Improvements Over Previous Draft
- Emphasizes not just auto-inserting `data.extract-site`, but actually **consuming** observation output to derive selectors and validate state (addressing recent Google/GitHub failures).
- Adds requirements for handling preconditions (consent dialogs, focus) and validation steps to catch mismatches before runtime.
- Tightens integration points (planner prompts, DSL schema, executor caching) so perception results remain accessible and referenced by later steps.

## Updated Work Items

### 1. Planner Prompt & Template Updates
- Instruct planners (LLM/rule) to *always* run an observation step first **and cite values** from that observation when constructing selectors.
- Provide sample selectors gleaned from observation (`headings`, `links`, `key_values`) and show how to translate them into `AgentLocator::Css` or `AgentLocator::Text`.
- Encourage planners to insert mitigation steps (e.g., click consent, wait for element) when observation reveals blocking UI.

### 2. DSL Enhancements
- Extend `AgentPlanStep.metadata` to reference observation fields (e.g., `"selector_hint": "${observe.links[0].url}"`).
- Add a `PlanContext` structure that stores the last observation blob so subsequent steps can interpolate values.
- Allow validations to assert expected identity/title from observation to detect wrong pages early.

### 3. Executor, Cache & Replan Feedback
- Cache the observation result on the task context and make it queryable by future steps (not just fallback).
- Surface the latest cached observation when `augment_request_for_replan` runs so the planner gets a short summary (see `latest_observation_summary` in `src/main.rs`). This keeps the LLM aware of the DOM snapshot that triggered the failure and is critical for selector re-generation.
- When a runtime failure occurs (“element missing”), check whether the observation suggested a different selector and include that hint in the error to aid replanning.

### 4. Toolchain & CLI UX
- `soulbrowser chat` keeps the new flags (`--perceive-first`, `--no-perceive-first`) but default templates enforce the perception-first behavior.
- Web console exposes the last observation result inline so users & LLM share the same context; add “Re-run observation” button when DOM drifts.

### 5. Testing/Validation
- Add integration tests simulating a page with consent modal + textarea (Google) to ensure planner outputs “Click consent → Type into textarea” based on observation.
- Regression tests ensuring autoplan adds observation even when the user template omits it, and that disabling the flag works.

### 6. Documentation
- README now mentions perception-first defaults and how to override them.
- Future docs (Quick Start / API) should include examples showing observation output snippets and how steps consume them.

## Next Steps
- Continue improving planner training/prompt so it references observation data when generating selectors.
- Consider storing “perception snapshots” in `TaskPlan` metadata for transparency/debugging.


## Outstanding Work

- None at the moment: `tests/l7_sanity.rs` now exercises the HTTP handler through the `run_tool_request_for_tests` helper and both `cargo test -p agent-core` + `cargo test --test l7_sanity` pass locally.
