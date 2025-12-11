# Backend Usage Quick Reference

The backend bring-up notes now live in a single troubleshooting playbook so you do not have to hop between multiple partially-updated docs.
Use this page for the bare minimum commands and jump to the canonical sources when you need deeper context.

> Looking for the legacy long-form walkthrough? See `docs/ARCHIVE/BACKEND_USAGE_LEGACY.md`.
>
> Need a step-by-step checklist? Follow `docs/guides/TROUBLESHOOTING.md` – it owns the serve/API debugging story going forward.

## 1. One-minute backend bring-up
```bash
# From the repo root
cargo run -- --metrics-port 0 serve --port 8789
```
Key env vars (optional):
- `SOULBROWSER_WS_URL` – re-use a Chrome instance (`ws://127.0.0.1:9222/devtools/browser/...`).
- `SOULBROWSER_DISABLE_PERCEPTION_POOL=1` – force fresh Chrome sessions when debugging pooling issues.
- `SOUL_RATE_LIMIT_BUCKET_TTL_SECS` – tune the in-memory rate limiter between demos.

When the server prints `Testing console available at http://127.0.0.1:8789`, the REST API and SSE stream are live.

## 2. Quick REST probes
```bash
curl http://127.0.0.1:8789/readyz
curl -X POST http://127.0.0.1:8789/api/perceive \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","mode":"all","timeout":45}'
```
Add `execute: true` in `/api/chat` payloads if you want the generated plan to run once; otherwise you only receive the structured plan/flow/artifact metadata.

## 3. Where to go next
- **Troubleshooting + deep dives** – `docs/guides/TROUBLESHOOTING.md`
- **Web console workflow** – `docs/guides/WEB_CONSOLE_USAGE.md`
- **Perception & parser development** – see the `parser_development` and `perception_service` sections inside `docs/README.md`

These three docs remain updated whenever Serve/API evolves. This quick reference stays intentionally short so we do not re-document the same flags in multiple locations.
