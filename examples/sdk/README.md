# SDK Usage Examples

This directory collects lightweight scripts that demonstrate how to interact with the
SoulBrowser APIs using the bundled SDKs.

## TypeScript examples
- `examples/sdk/typescript/chat_plan.mjs`
  - Requirements: build the TypeScript SDK (`cd sdk/typescript && npm install && npm run build`)
  - Install `ws`: `npm install ws`
  - Run:
    ```bash
    node examples/sdk/typescript/chat_plan.mjs "打开 https://example.com 并截图"
    ```
    Calls `/api/chat` with `execute=true`, prints the generated plan, and listens to
    `overlay` events via WebSocket.
- `examples/sdk/typescript/observations.mjs`
  - Same requirements as above.
  - Run (task id optional):
    ```bash
    node examples/sdk/typescript/observations.mjs <task_id?> [limit]
    ```
    Fetches `/api/tasks/:id/observations` and prints the latest observation history,
    including bounding boxes and screenshot paths.

## Python examples
- `examples/sdk/python/chat_plan.py`
  - Requirements: install the SDK (`pip install -e sdk/python[ws]`)
  - Run:
    ```bash
    python examples/sdk/python/chat_plan.py --prompt "打开 https://example.com 并截图"
    ```
    Mirrors the TypeScript chat example: creates a task and streams overlay events.
- `examples/sdk/python/observations.py`
  - Same requirements as above.
  - Run:
    ```bash
    python examples/sdk/python/observations.py <task_id?> [limit]
    ```
    Prints the observation timeline for a task using the SDK’s
    `get_task_observations` helper.
- `examples/sdk/python/recordings.py`
  - Run without arguments to list the latest recordings, or pass a session id to
    inspect metadata embedded during `soulbrowser record`:
    ```bash
    python examples/sdk/python/recordings.py [session_id]
    ```
    Uses the new `list_recordings` / `get_recording` APIs.

Both scripts assume the backend is running at `http://127.0.0.1:8801`. Override by
setting `SOULBROWSER_API_BASE`.
