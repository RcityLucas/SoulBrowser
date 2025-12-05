# Flexible Parser & Tool Expansion Plan

## 1. Objectives
- Ensure SoulBrowser can quickly satisfy user requests for new data sources (e.g., Twitter, LinkedIn, niche sites).
- Provide a repeatable way to add deterministic parsers + schemas without breaking existing plans.
- Align planner, executor, and delivery surfaces so newly added tools are immediately usable from CLI and Web Console.

## 2. Scope & Assumptions
- Focus on read-only data extraction/structuring. Write actions (posting, liking) are out of scope.
- Parser implementations are in Rust and emit structured JSON via `data.deliver.*`.
- Tool registry is tenant-scoped; changes must work for both CLI and server deployments.
- We assume Chrome CDP observation (`data.extract-site`) remains the primary raw data source; API-based tools are optional extensions.

## 3. Workstreams

### 3.1 Tooling & Schema Audit
1. Enumerate existing `data.parse.*` and `data.deliver.*` usages (grep + registry listing).
2. Rank top requested sites (GitHub/Twitter done; next candidates: LinkedIn profiles, news portals, Product Hunt / Reddit digests).
3. For each target, capture desired schema fields, example pages, data sensitivity, and rate limits.
4. Output: `docs/reference/schema_catalog.md` table listing schema id, purpose, parser module, and release owner.

### 3.2 Parser Template & Scaffolding
1. Create a `cargo xtask parser new <name>` (or `scripts/new_parser.rs`) scaffold that generates:
   - `src/parsers/<name>.rs` with placeholder functions, tests, and schema constants.
   - `docs/reference/schemas/<name>.json` describing output contract.
   - Sample deliver config referencing artifact labels/screenshots.
2. Provide helper utilities:
   - DOM querying helpers (XPath/CSS reducers, text normalizers).
   - Common metadata builder (record_count, source_url, fetched_at, hero image).
3. Document process in `docs/guides/PARSER_DEVELOPMENT.md`.

### 3.3 Planner Alignment
1. Update `src/llm/prompt.rs` instructions: list canonical tool ids (e.g., `data.parse.twitter-feed`).
2. Extend `PlanValidator` to reject unknown custom-tool names and explain allowed alternatives.
3. Maintain alias map in `ChatRunner::normalize_custom_tools` for legacy ids (similar to GitHub fix) plus unit tests to guard regressions.
4. Add synthetic planner tests (golden plans) verifying that prompt templates generate the required `navigate -> data.extract-site -> data.parse.* -> data.deliver.*` sequence for new schemas.

### 3.4 Executor & Runtime Support
1. Expand `StepOutputsState` alias handling so any `github.extract-repo`-style legacy name maps to the proper parser.
2. Add telemetry counters:
   - `planner.tool_alias_hit` when a legacy name is rewritten.
   - `executor.unsupported_custom_tool` when a plan bypasses validation.
3. Ensure `handle_parse_*` can select an observation by explicit step id (payload override) to support multi-page workflows.
4. Provide feature flag / config to disable specific parsers per deployment (policy center integration).

### 3.5 Delivery & Surfaces
1. Define schema manifests (id, description, columns) consumed by:
   - Task Center artifact viewer (table rendering, screenshot preview).
   - `soulbrowser artifacts --extract` CLI command for CSV export.
2. Update Web Console to group structured outputs by schema, show row count, and provide download links.
3. Add automated changelog snippets referencing new schemas when delivered artifacts appear.

### 3.6 API / Custom Tool Integration (Optional)
1. For sites with hostile DOMs (Twitter), implement API-backed tools:
   - Extend `src/tools.rs` with `data.fetch.twitter` that wraps official API, handles OAuth tokens stored in policy center.
   - Provide matching parser that normalizes API JSON into the standard schema.
2. Document environment variables / secrets management for these integrations.

### 3.7 Validation & Observability
1. Add integration tests per parser (headless) using cached observation fixtures in `tests/fixtures/observations/`.
2. Create contract tests ensuring `data.deliver.*` artifacts meet schema (JSON Schema validation step).
3. Update metrics dashboards (Grafana or `docs/metrics/structured_outputs.md`) with:
   - Count of delivered schemas per day.
   - Failure reasons (no observation, parse error, deliver conflict).

### 3.8 Rollout Plan
1. **Phase 0 (Week 1):** Implement scaffolding, aliasing, and documentation. Ship GitHub fix (done) + sample new parser (HackerNews feed now live) as proof of concept.
2. **Phase 1 (Weeks 2-3):** Add first high-demand parser (Twitter read-only timeline). Enable behind feature flag (`SOULBROWSER_ENABLE_TWITTER_PARSER`).
3. **Phase 2 (Weeks 4-5):** Expand to LinkedIn/News; integrate API-backed tool if rate limits block DOM scraping.
4. **Phase 3 (Continuous):** Monthly schema audit, add new requests via lightweight RFC process.

## 4. Risk & Mitigations
| Risk | Impact | Mitigation |
| --- | --- | --- |
| DOM layout changes break parsers | High | Use robust selectors + fallback heuristics, add monitoring for parse error spikes. |
| Planner generates unsupported tool names | Medium | Strict validation + alias map + CI tests on plan templates. |
| API quota limits for sites like Twitter | Medium | Implement caching, require operator-provided tokens, fall back to DOM extraction when possible. |
| Structured schema drift | Medium | Central schema catalog + JSON Schema validation during deliver stage. |

## 5. Deliverables
- Parser scaffolding tooling + guide.
- Schema catalog documentation.
- Updated planner prompt/validator with alias coverage.
- Executor telemetry + alias handling (GitHub + future parsers).
- Web Console + CLI enhancements for structured artifact viewing/export.
- Integration + contract test suites.
- Feature-flagged rollout for at least one new parser (Twitter).

## 6. Success Metrics
- <5% of structured tasks fail due to “unsupported tool” or “no parsed output”.
- Time to add a new parser (schema + deliver) < 1 day after spec.
- Structured artifact downloads increase (track via Task Center telemetry).
- Positive operator feedback: ability to fulfill new user asks without core changes.
