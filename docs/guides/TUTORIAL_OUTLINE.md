# SoulBrowser Tutorial Outline

This document tracks the assets promised in **Phase 5 · Polish & Documentation** of `INTEGRATION_ROADMAP.md`. Each section includes the storyboard, required recordings, and the CLI/API snippets to capture once screen recording starts.

---

## 1. Quick Start — "自然语言 → 浏览器自动化"

- **Goal**: Show a new user how to translate a natural-language goal into an executable plan with automatic replanning.
- **Script**:
  1. Open Terminal, export `SOULBROWSER_OPENAI_API_KEY` (or use mock provider) and run `cargo run -- chat --planner llm --llm-provider mock --prompt "查找 SoulBrowser 的最新 release" --execute --max-replans 1`.
  2. Highlight CLI output (plan summary, execution attempts, replanning log lines).
  3. Switch to Task Center, show the new task row and live status/log streaming.
- **Artifacts to capture**: 1080p screen recording, optional voice-over, commands pasted into README/QUICK_START.

## 2. Task Center Walkthrough — "管理/重试/取消"

- **Goal**: Teach operators how to inspect historical runs, stream logs, and perform re-execution or cancellation directly from the UI.
- **Script**:
  1. Start from `/tasks` route; explain table columns (prompt, planner, LLM provider/model, status).
  2. Open a task drawer; scroll through plan JSON, dispatch table, and live logs.
  3. Use the **重新执行** button to trigger `/api/tasks/:id/execute`, watch status update, and export logs.
  4. Showcase WebSocket disconnect handling (toggle server/reconnect) to demonstrate resilience.
- **Artifacts**: Short GIF of drawer interactions, narrated video (≤3 min), update docs with screenshots.

## 3. LLM Configuration Deep Dive — "从密钥到多 Provider 切换"

- **Goal**: Explain environment variables, CLI flags, and Task Center creation form for switching planners/providers.
- **Script**:
  1. Walk through `.env` / shell export for OpenAI + Claude.
  2. Demonstrate `POST /api/tasks` payload that sets `planner=llm`, `llm_provider=openai`, `llm_model=gpt-4o-mini`, and `execute=true`.
  3. In the UI, open the **新建任务** modal, select “LLM” and show form validation, then submit and monitor execution.
- **Artifacts**: Slides or annotated screenshots of the modal & API payload; optional short video.

---

**Next Steps**

1. Capture raw footage during the next developer run and drop assets under `docs/tutorial-assets/`.
2. Update `README.md` / `QUICK_START.md` once recordings are uploaded.
3. Share download links in the release notes for the upcoming MVP drop.

