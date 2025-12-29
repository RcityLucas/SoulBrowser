# L0 CDP Adapter Hardening Plan

The `cdp-adapter` crate still behaves like a scaffold in several key areas. This checklist captures
what remains before we can treat it as production-ready.

## Gap Assessment

1. **Transport selection silently falls back to a noop stub.** By default the adapter only enables
   the real Chromium transport when `SOULBROWSER_USE_REAL_CHROME=1`, otherwise every command routes
   to `NoopTransport` and immediately fails (`crates/cdp-adapter/src/lib.rs:631-662`). We now
   attempt to auto-detect Chrome and only fall back to the stub when it truly is unavailable.
2. **Chrome discovery previously panicked.** When the executable could not be found we would
   `panic!` instead of surfacing an actionable error (`crates/cdp-adapter/src/lib.rs:636-644`). The
   new logic emits a warning and keeps the CLI responsive.
3. **There are no contract tests exercising `CdpAdapter` end-to-end.** The existing integration
   tests only cover `ChromiumTransport`; higher-level operations such as `navigate`, `click`,
   `type_text`, and screenshot capture are verified solely through unit tests with mocked
   transports. An external contract test (even marked `#[ignore]`) is needed to validate the real
   adapter wiring.
4. **Event-loop wiring is still marked as “pending”.** Even though we collect real CDP events the
   event loop currently just logs `"event loop started (real CDP wiring pending)"`
   (`crates/cdp-adapter/src/lib.rs:803-808`). The events are not forwarded to StateCenter or any
   downstream observer, so structural perceivers cannot yet rely on them.

## Next Actions

1. **Finalize transport detection (DONE in this pass).** Auto-detect Chrome and downgrade to the
   stub only when discovery fails or the user explicitly disables it.
2. **Add contract smoke tests.** Create an ignored integration test exercising
   `CdpAdapter::navigate`/`click` against a real Chrome instance so we can validate the combined
   adapter instead of just the transport.
3. **Wire CDP events into StateCenter.** Replace the placeholder event-loop logging with real calls
   into the structural perception pipeline so DOM/page lifecycle events become observable by higher
   layers.
4. **Document the supported environment variables.** README/docs should spell out
   `SOULBROWSER_USE_REAL_CHROME`, `SOULBROWSER_CHROME`, and the new auto-detection behavior.
