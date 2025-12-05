# Parser Development Guide

This guide explains how to add a new deterministic parser + schema so SoulBrowser can deliver structured artifacts for additional sites (Twitter, LinkedIn, etc.).

## 1. Generate scaffolding

Use the built-in helper to create a parser module + JSON schema stub:

```
cargo run --bin parser_scaffold -- twitter-feed \
  --title "Twitter Feed" \
  --description "Normalized timeline items"
```

This command will:

1. Create `src/parsers/twitter_feed.rs` with a placeholder `parse_twitter_feed` function and TODOs.
2. Append the necessary `pub mod`/`pub use` statements to `src/parsers/mod.rs`.
3. Generate `docs/reference/schemas/twitter_feed_v1.json` with a JSON Schema stub you can refine.
4. Produce `docs/reference/schemas/twitter_feed_v1_deliver.md` containing a ready-to-tweak `data.deliver.structured` payload (artifact label + screenshot hints).
5. Print follow-up instructions (update planner prompts, add tests, etc.).

> Tip: pass `--schema-id twitter_feed_v2` or `--force` if you need a specific schema name or want to overwrite existing files.

## 2. Implement the parser

- Populate the placeholder with real extraction logic (look at `market_info.rs` / `news_brief.rs` for patterns).
- Use helper crates (e.g., `regex`, `url`) as needed.
- Lean on `src/parsers/helpers.rs` for observation metadata utilities (`extract_observation_metadata`, `text_from_candidates`, `normalize_whitespace`, etc.) so new parsers don't re-implement boilerplate URL/title handling.
- Return `serde_json::Value` matching the schema defined in `docs/reference/schemas/...`.

## 3. Wire the deliver step

- Ensure the planner references your parser using `data.parse.<name>` (update `src/llm/prompt.rs` & `ChatRunner::normalize_custom_tools`).
- In the executor, route the tool name to the parser function (see `handle_parse_github_repos`).
- Deliver via `data.deliver.structured` with the schema id (start from the scaffolded `docs/reference/schemas/<schema>_deliver.md` snippet and adjust labels/paths as needed).

## 4. Document + test

- Update [`docs/reference/schema_catalog.md`](../reference/schema_catalog.md) with the new schema.
- Add integration/unit tests for the parser using fixtures under `tests/fixtures/observations/` (if applicable).
- Run `cargo fmt` and `cargo test parser::<name>` (or the relevant module).

Following this loop keeps SoulBrowser ready to answer new “fetch data from X” requests with minimal friction.
