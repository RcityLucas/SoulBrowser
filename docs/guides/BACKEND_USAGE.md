# Backend Usage Guide

This guide pulls together the scattered pieces of documentation into a single
reference for running and exercising the SoulBrowser backend.

## 1. Prerequisites

- Rust toolchain (1.72 or newer suggested)
- Google Chrome / Chromium with DevTools remote debugging support
- Optional: Node.js (only if you plan to run the web console)

### Chrome options

| Environment | Recommendation |
| ----------- | -------------- |
| Windows     | Use the bundled Chrome (`C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe`). |
| WSL         | Launch Chrome on Windows with `--remote-debugging-port=9222` and set `SOULBROWSER_WS_URL=http://127.0.0.1:9222`. |
| Linux/macOS | Let the adapter auto-detect, or export `SOULBROWSER_CHROME=/path/to/chrome` if necessary. |

Useful environment flags:

```bash
export SOULBROWSER_USE_REAL_CHROME=1       # force real CDP transport
export SOULBROWSER_DISABLE_SANDBOX=1       # work around sandbox restrictions (WSL/CI only)
export SOULBROWSER_WS_URL=http://127.0.0.1:9222   # reuse an existing Chrome session
```

## 2. Start the backend service

Run once per machine and keep the process alive:

```bash
cargo run -- --metrics-port 0 serve --port 8789
```

Key output:

```
Testing console available at http://127.0.0.1:8789
Using local Chrome detection (SOULBROWSER_CHROME / auto-detect)
```

Troubleshooting:

- "failed to bind" → the port is in use. Pick a new one via `--port 8790` or kill the resident process (`netstat -ano | find "8789"`).
- "chromium exited before exposing devtools websocket" → Chrome failed to launch; check sandbox flags or WS URL.

## 3. CLI quick checks

```bash
# Structural + visual + semantic perception
cargo run -- --metrics-port 0 perceive --url https://example.com --timeout 60 --all --insights

# Structural-only fast probe
cargo run -- --metrics-port 0 perceive --url https://example.com --structural
```

Results print to stdout and, when requested, JSON / screenshots are written via
`--output` or `--screenshot`.

Temporary Chrome user-data directories (`.soulbrowser-profile-*`) are created in
the repo root. They are removed once Chrome exits; if not, run
`scripts/cleanup_profiles.sh` (Linux/macOS/WSL) or `scripts/cleanup_profiles.ps1`
(Windows).

> 默认行为：无论 Planner 是否显式添加采集步骤，计划执行成功后都会自动运行一次 `data.extract-site`（即 `page.observe`），把当前页面的结构化 JSON 写入 `soulbrowser-output/artifacts/<task_id>/`。失败路径也会触发同样的兜底观察，方便排查。

### CLI ↔ REST 对照

| 使用场景 | CLI 命令 | REST 调用 |
| -------- | -------- | --------- |
| 健康检查 | — | `GET /health` |
| 全量感知 | `cargo run -- ... perceive --url <URL> --all --insights` | `POST /api/perceive` with `{"mode":"all"}` |
| 结构化-only | `... perceive --structural` | `POST /api/perceive` + `{"mode":"structural"}` |
| Chat 规划 | `cargo run -- chat --prompt "..."` | `POST /api/chat` |

## 4. REST API calls

With the service running on `127.0.0.1:8789`:

```bash
curl -X POST http://127.0.0.1:8789/api/perceive \
     -H "Content-Type: application/json" \
     -d '{"url":"https://example.com","mode":"all","timeout":60,"insights":true}'
```

Optional fields: add a `viewport` object to override device metrics, a `cookies` array to seed authenticated sessions, or an `inject_script` string to run custom JavaScript right after DOM Ready.

Other useful endpoints:

- `GET /health` – quick readiness probe
- `POST /api/chat` – invokes the L8 rule-based planner (set `execute: true` to run the flow)
- `GET /api/tasks` – list recent scheduler/dispatch activity for tasks
- `GET /api/tasks/{task_id}` – view dispatch history for a specific task
- `POST /api/tasks` – returns 501 (placeholder until task submission is implemented)

Example chat request:

```bash
curl -X POST http://127.0.0.1:8789/api/chat \
     -H "Content-Type: application/json" \
     -d '{"prompt":"open rust-lang.org and take a screenshot","execute":false}'
```

Response excerpt:

```json
{
  "success": true,
  "plan": {
    "plan": { "steps": [ { "title": "Navigate to rust-lang.org", "id": "step-1" }, ... ] },
    "explanations": ["Step 1: navigate", "Step 2: capture screenshot"]
  },
  "flow": {
    "metadata": { "step_count": 3, "validation_count": 0 },
    "definition": { "root": { "Sequence": { ... } } }
  }
}
```

When the API call hangs, inspect the serve log; the backend writes child process
stdout/stderr and the error message (e.g. DOM snapshot failures).

## 5. Web console (optional)

```
# Terminal A: backend
cargo run -- --metrics-port 0 serve --port 8789

# Terminal B: frontend
(cd web-console && npm install && npm run dev)
```

The Vite dev server proxies `/api` to the backend port. Access
http://localhost:5173 to exercise the UI.

## 6. Housekeeping

- Run `scripts/cleanup_profiles.sh` or `.ps1` before committing to keep the repo tidy.
- Use `scripts/clean_output.sh` / `.ps1` to remove `soulbrowser-output/`, `tmp/`, and stale `plan*.json` artifacts after demos or test runs.
- `cargo fmt && cargo test` keeps Rust code style/tests green.
- WSL users should periodically close stray `chrome.exe` processes to free profile locks.

## 7. Additional references

- `docs/guides/TROUBLESHOOTING.md` – canonical playbook for backend/frontend/runtime issues.
- `docs/guides/WEB_CONSOLE_USAGE.md` – step-by-step UI walkthrough once the backend is up.
- `docs/reference/L2_OUTPUT_REFERENCE.md` – perception payload schema.
