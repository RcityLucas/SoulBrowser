# L8 Agent Core Overview

This document captures the first delivery slice for the L8 "Agent & Experience" layer.

## Components

- **`crates/agent-core`**: foundational data models (`AgentPlan`, `AgentToolKind`),
  a rule-based planner (`RuleBasedPlanner`), and helpers to convert plans into
  `action-flow` structures.
- **`src/agent/mod.rs`**: CLI-facing chat runner that wraps the planner,
  provides summaries, scroll heuristics, and exposes serialized plan/flow artifacts.
- **`soulbrowser chat`**: new command that accepts a natural language prompt,
  optional constraints/context, prints the planned execution steps, and (with
  `--execute`) runs them through the scheduler/toolchain with per-step retries
  (`--max-retries`), re-planning attempts (`--max-replans`), dispatch latency
  metrics, and serialized run artifacts (`--save-run`) that include scheduler
  timelines and State Center events.

## MVP Flow

1. User runs `soulbrowser chat --prompt "Open https://example.com and click 'Pricing'"`.
2. `ChatRunner` builds an `AgentRequest`, bootstraps rule-based planner heuristics.
3. Planner emits `AgentPlan` with navigation + click steps and validations.
4. `plan_to_flow` converts the plan into `Flow` for `action-flow` execution.
5. CLI prints human-readable steps; optional JSON/YAML export via `--output json`.

## Next Steps

- Expand planner heuristics (form filling, table extraction, pagination).
- Introduce memory/profile hints once L4 `memory-center` lands.
- Connect generated flows to scheduler for live dry-runs.
- Add UI surface (web console) showing the same plan/flow artifacts.
