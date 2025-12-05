# CDP-backed Action Primitives Plan

## Goals
- Drive real browser state for `navigate`, `click`, and `type-text` primitives instead of returning placeholder success.
- Ensure automatic `data.extract-site` / `page.observe` captures real pages (no more `about:blank`).
- Provide clear fallbacks when the adapter runs in Noop mode so tasks fail fast instead of pretending to succeed.

## Work Items

### 1. Adapter Surface & Routing
- Expose public async helpers on `CdpAdapter` that mirror the `Cdp` trait (`navigate`, `query`, `click`, `type_text`, `evaluate_script`).
- Share route→page mapping logic between `BrowserToolExecutor` and action primitives so both pick a live page/frame tied to the ExecRoute.
- Detect Noop transport up-front; surface a structured error if real Chrome isn’t enabled.

### 2. Navigation Primitive
- Replace the TODO in `crates/action-primitives/src/primitives/navigate.rs` with:
  1. Resolve `PageId` from `ExecCtx`.
  2. Call `adapter.navigate(page, url, ctx.remaining_time())`.
  3. Apply the existing wait tier via `DefaultWaitStrategy`.
  4. Use `adapter.evaluate_script` to capture `window.location.href` and `document.title` for `PostSignals`.
- Propagate adapter errors so LLM can replan when navigation fails (network, bad URL, etc.).

### 3. Click & Type Primitives
- Implement selector resolution by calling `adapter.query` with CSS/Text/ARIA options derived from `AnchorDescriptor`.
- Execute clicks using either `adapter.click` or a Runtime script fallback; include wait-tier handling.
- Implement typing by focusing the element, setting `value`, and optionally sending `Enter` when `submit = true`.
- Capture updated post signals after each action.
- Add rudimentary self-heal retries (e.g., fallback from CSS to text search) and surface the chosen selector in `ActionReport` metadata.

### 4. Executor & Planner Touchpoints
- Update tool execution (e.g., `src/tools.rs` `navigate-to-url`, `click`, `type-text`) to require execution routes and bubble adapter errors.
- Ensure fallback observe (`src/agent/executor.rs:595-652`) keeps using the last successful route so it captures the page we just interacted with.

### 5. Testing & Validation
- Add unit tests for the new primitives using a stub `CdpAdapter::with_transport` that records method calls.
- Re-enable real-browser smoke tests behind `SOULBROWSER_USE_REAL_CHROME=1` (`cargo test -p cdp-adapter` and `cargo run -- demo`).
- Manual regression: run the GitHub avatar/name task and confirm artifacts point to `https://github.com/<user>` with populated headings/identity fields.

## Risks & Mitigations
- **Real Chrome dependency**: document the need for `SOULBROWSER_USE_REAL_CHROME=1` in README and fail fast when missing.
- **Selector brittleness**: start with CSS selectors but keep hooks ready for AX/Text strategies; log attempts for observability.
- **Performance**: cache route→page mappings and reuse adapter sessions to avoid reconnect churn.
