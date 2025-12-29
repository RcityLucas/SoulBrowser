# Visual Testing Console Usage Guide

This guide explains how to launch the lightweight SoulBrowser testing server (`serve` subcommand) and interact with the new web-based console for running perception tests against a real Chrome/Chromium instance.

## 1. Prerequisites

- **Chrome/Chromium binary**: either accessible on the current machine or available through a remote DevTools endpoint.
- **Rust build**: run `cargo build --release` once so the `soulbrowser` binary is available under `./target/release/`.
- (Optional) `SOULBROWSER_WS_URL`: if set, the server attaches to an existing Chrome DevTools URL instead of launching a browser itself. This is useful in WSL or sandboxed environments.

## 2. Starting Chrome on Windows (for WSL users)

In PowerShell or Command Prompt, launch Chrome on the host with a remote debugging port:

```powershell
"C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-port=9222 --user-data-dir=C:\ChromeRemote
```

Keep this window open while you run the testing server in WSL.

## 3. Launching the Testing Server

### Option A – 使用封装脚本

```bash
cd /mnt/d/github/SoulBrowserClaude/SoulBrowser
./scripts/run_visual_console.sh            # 自动检测 Chrome
./scripts/run_visual_console.sh --port 8800
./scripts/run_visual_console.sh --ws-url http://127.0.0.1:9222
```

首行命令会自动编译（若需要）、检测常见 Chrome/Chromium 安装路径并启动测试服务。`--ws-url` 用于连接外部 DevTools 端口，例如 Windows 上的 Chrome。

### Option B – 手动运行现有可执行文件

```bash
cd /mnt/d/github/SoulBrowserClaude/SoulBrowser
./target/release/soulbrowser --metrics-port 0 serve --port 8787
```

### Option C – Attach to an external DevTools endpoint (e.g. Windows Chrome)

```bash
cd /mnt/d/github/SoulBrowserClaude/SoulBrowser
export SOULBROWSER_WS_URL=http://127.0.0.1:9222
./target/release/soulbrowser --metrics-port 0 serve --port 8787 --ws-url http://127.0.0.1:9222
```

Notes:
- `--metrics-port 0` disables the default Prometheus binding to avoid permission issues.
- If you prefer building in debug mode, replace `./target/release/soulbrowser` with `./target/debug/soulbrowser`.
- The server prints the URL of the console once it starts (default `http://127.0.0.1:8787`).
- When the backend runs inside WSL but you open the console from Windows, add `--host 0.0.0.0` (or any reachable address) so the HTTP server listens on all interfaces and Windows can reach the API, including the `/api/sessions` endpoints.

## 4. Using the Web Console

1. Open the URL shown in the terminal (e.g. `http://localhost:8787`).
2. Enter the target URL (default is `https://example.com`).
3. Choose the perception mode:
   - Select one of the presets (All/Structural/Visual/Semantic).
   - Pick **Custom selection** if you want to toggle Structural/Visual/Semantic individually – the extra checkboxes appear only in this mode.
   - Toggle “Insights” or “Capture screenshot” as needed.
4. Click **Run Perception**.
5. The results page displays:
    - A status banner showing run progress or errors in real time
    - Summary cards for Structural/Visual/Semantic metrics plus cross-modal insights
    - Raw perception JSON (expand the “Raw JSON payload” section when you need full detail)
    - `stdout`/`stderr` from the `soulbrowser perceive` command
    - A screenshot preview (and download link) when capture is enabled
    - A “Recorded timeline” and “Artifacts” viewer populated whenever you load a saved run bundle via `soulbrowser console --input <bundle>`

### Console panels at a glance

- **Status banner** – reflects the latest request state (`Idle`, `Running`, `Success`, `Warning`, `Error`).
- **Latest Perception** – cards summarising structural, visual, and semantic metrics, plus an expandable JSON dump.
- **Runtime logs** – streamed stdout/stderr output from the spawned `soulbrowser perceive` process.
- **Screenshot** – renders the captured PNG and offers a download link whenever screenshot capture is enabled.
- **Recorded timeline & Artifacts** – shows State Center events and captured artifacts when viewing saved run bundles (via `soulbrowser console --input ...`).

If the request fails, the console shows the error message plus the raw STDOUT/STDERR for debugging. Common causes include missing Chrome permissions or invalid URLs.

### Fixture mode for automated testing

When you need to exercise the console API without a live Chrome session (for example in CI), set `SOULBROWSER_CONSOLE_FIXTURE=/path/to/fixture.json` before launching `soulbrowser serve`. The fixture file should contain a serialized `MultiModalPerception` payload plus optional `stdout`, `stderr`, `success`, and `screenshot_base64` fields. The server will return this fixture instead of spawning a browser, keeping `/api/perceive` responsive in headless test environments. You can also add `SOULBROWSER_CONSOLE_FIXTURE_SCREENSHOT=/path/to/image.png` to override the screenshot base64 with the contents of an actual PNG.

## 5. Troubleshooting

| Symptom | Possible Causes | Suggested Fix |
|---------|-----------------|---------------|
| `multi-modal perception failed` | Chrome cannot start (sandbox restrictions) | Use `SOULBROWSER_WS_URL` to attach to an external Chrome; ensure `SOULBROWSER_DISABLE_SANDBOX=1` is set |
| `chromium exited before exposing devtools websocket url` | Browser terminated before CDP handshake | Start Chrome manually with `--remote-debugging-port`; verify permissions |
| Screenshot missing | Capture disabled or file not written | Ensure “Capture screenshot” is checked; check STDOUT/STDERR for errors |
| Metrics binding error | Port 9090 blocked or restricted | `--metrics-port 0` already disables metrics; ignore the warning |

## 6. Optional: CLI Script for Automated Testing

The existing shell script `tests/l8_visual_suite.sh` still works for CI/automation and now honours `SOULBROWSER_WS_URL`. Use it when you need scripted validation rather than the web UI.

```bash
# Example using external Chrome at 9222
SOULBROWSER_WS_URL=http://127.0.0.1:9222 ./tests/l8_visual_suite.sh
```

---
With the testing server and web console you can iterate on perception features, visual anchors, and future L8 agent flows without leaving the browser.
