# Troubleshooting Playbook

Use this checklist to diagnose the most common issues when running SoulBrowser locally. Symptoms are grouped by subsystem so you can scan quickly.

## 1. Backend fails to start
- **Port already in use** – Run `netstat -ano | findstr 8787` (Windows) or `lsof -i :8787` (Linux) and kill the conflicting process, or restart with `cargo run -- --port 8790`.
- **Chrome path not found** – Set `SOULBROWSER_CHROME` to the absolute path or launch Chrome manually with `--remote-debugging-port=9222` and point `SOULBROWSER_WS_URL` to the resulting WebSocket URL.
- **Permission denied (WSL)** – Export `SOULBROWSER_DISABLE_SANDBOX=1` so Chrome can run inside the containerized environment.

## 2. Perception errors (`multi-modal perception failed`)
1. Confirm Chrome actually starts: `tasklist | findstr chrome.exe` (Windows).
2. Remove stale temporary profiles:
   ```bash
   ./scripts/cleanup_profiles.sh        # bash
   powershell -File scripts/cleanup_profiles.ps1  # Windows
   ```
3. Retry with `SOULBROWSER_USE_REAL_CHROME=1` to force the CDP adapter into real-browser mode.
4. Collect logs: rerun with `RUST_LOG=debug cargo run -- perceive ...` and inspect `soulbrowser-output/logs/latest.log`.

## 3. Task Center shows "Service unavailable"
- Ensure the backend health endpoint returns 200: `curl http://127.0.0.1:8787/health`.
- If you run the frontend on a different host/port, set `VITE_API_BASE_URL` (example: `http://127.0.0.1:8789`).
- Browser console `CORS` errors indicate the backend URL is misconfigured.

## 4. Chat/Agent stuck on "thinking"
- Verify your LLM key is loaded (`soulbrowser info` prints provider status).
- Temporarily switch to the rule-based planner: `soulbrowser chat --planner rule-based --prompt ...` to confirm the scheduler works.
- Check the streaming endpoint with `curl -N http://127.0.0.1:8787/api/tasks/<task_id>/stream` and look for `error` events.

## 5. Artifacts missing screenshots
- Confirm `SOULBROWSER_USE_REAL_CHROME=1`.
- Ensure `soulbrowser-output/` is writable; if running in CI/WSL, mount the repo with read-write permissions.
- Use `cargo run -- demo --screenshot verify.png` to test screenshot capture in isolation.

## 6. Cleanup checklist
- `scripts/cleanup_profiles.*` removes `.soulbrowser-profile-*` dirs.
- `scripts/dev_checks.sh` runs fmt/tests before committing.
- `git clean -fdx web-console/node_modules` fixes broken node installs.

For deeper dives refer to:
- Backend quick reference + command cheatsheet: `docs/guides/BACKEND_USAGE.md`
- Legacy full walkthrough (if you need the old flow): `docs/ARCHIVE/BACKEND_USAGE_LEGACY.md`
- Web console usage: `docs/guides/WEB_CONSOLE_USAGE.md`
- Task Center schema: `docs/reference/api_TASK_CENTER.md`

If you hit an unlisted issue, open `soulbrowser-output/logs/latest.log` and include it when filing a bug.
