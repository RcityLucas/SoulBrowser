# Security Notes

Practical guidance for running SoulBrowser securely in dev and demo environments.

## 1. Secrets & Environment
- **Never hard-code API keys** in code or configs. Put them in `config/local.env` (already `.gitignore`d)
  or your shell profile, then start the backend. Example:
  ```
  SOULBROWSER_OPENAI_API_KEY=sk-...
  SOULBROWSER_CLAUDE_API_KEY=sk-ant-...
  ```
- Production deployments should use OS-level secret stores (systemd `EnvironmentFile`, Kubernetes
  Secrets, GitHub Actions `secrets`, etc.).
- Keep `/env`-style files out of version control and access-controlled—`dotenvy` will load them
  automatically, so do not log them to the console.

## 2. Network binding & ports
- The backend listens on `127.0.0.1` by default. Use `--port` plus a reverse proxy if you must expose it
  publicly; never bind to `0.0.0.0` without TLS/auth in front.
- Metrics (Prometheus) defaults to 9090. Either restrict it to localhost or disable via
  `--metrics-port 0` if not needed.

## 3. Rate limiting & abuse protection
- Built-in knobs: `SOULBROWSER_RATE_LIMIT_CHAT` and `SOULBROWSER_RATE_LIMIT_TASKS`. Keep them non-zero
  when running on shared networks.
- Frontend surfaces (Tasks page) only call the same REST endpoints; ensure browsers access them via HTTPS
  if tunneling over the internet.

## 4. Browser automation safety
- Headless mode is default. If you run headful (`SOULBROWSER_HEADFUL=1`), ensure `SOULBROWSER_CHROME`
  points to a trusted binary and `SOULBROWSER_DISABLE_SANDBOX=1` is only used in controlled
  environments (e.g., CI or Windows dev boxes).
- When attaching (`--ws-url`) to an existing Chrome, run Chrome with a dedicated profile directory to
  avoid leaking your personal cookies/history into automation logs.

## 5. WebSocket / Task Stream
- `/api/tasks/:id/stream` pushes live logs; treat it like any other authenticated API if you expose it.
  Put it behind a reverse proxy that enforces HTTPS and optionally auth headers.
- The log payload may include user-entered text (Form inputs, prompts). Redact or filter before showing in
  multi-tenant dashboards.

## 6. Logging & storage
- `soulbrowser-output/tasks/*.json` contains prompts, plan metadata, and LLM provider names. Restrict file
  permissions in shared servers (`chmod 700` on Linux, NTFS ACLs on Windows).
- Use log rotation or `RUST_LOG` filtering to avoid dumping sensitive data in CLI output when running in CI.

## 7. Serve authorization & fault injection
- Serve enables token/IP-based auth by default. When you run `cargo run -- serve`, the CLI prints a
  generated token (set via `--auth-token`, `SOUL_CONSOLE_TOKEN`, or `SOUL_SERVE_TOKEN`). Only requests that
  present `x-soulbrowser-token` or `Authorization: Bearer` with a valid token and originate from the
  whitelist (`127.0.0.1` / `::1` unless overridden by `--allow-ip`) are accepted. Set `SOUL_STRICT_AUTHZ=1`
  (automatically set when any auth token is configured) to force the policy adapter into locked-down mode.
- Avoid `--disable-auth` except on an isolated loopback interface. If you must expose the Serve API through
  a tunnel/proxy, terminate TLS and revalidate tokens there.
- Testing the self-heal pipeline no longer requires custom scripts: call
  `POST /api/self-heal/strategies/:id/inject` (with Serve auth headers) to record a synthetic
  `SelfHealEvent`. This triggers the webhook (`SOULBROWSER_SELF_HEAL_WEBHOOK`) and writes to the Task Stream
  so you can verify alerting pipelines. Only expose this endpoint to trusted operators; it can flood alerts
  if misused.

Keeping these guardrails in place helps the Phase 5 “Polish & Security” checklist stay green.
