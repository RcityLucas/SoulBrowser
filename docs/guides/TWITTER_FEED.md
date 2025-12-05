# Twitter Feed Extraction Guide

This walkthrough demonstrates how to use SoulBrowser to collect a Twitter/X timeline and deliver structured `twitter_feed_v1` output.

## 1. Prerequisites
-
- Chrome/Chromium accessible to the CLI.
- Valid OpenAI/Claude API keys if you rely on the LLM planner (rule-based works too).
- `SOULBROWSER_USE_REAL_CHROME=1` (unless you connect to an existing remote debugging session).

## 2. Launch a task from the CLI

```
SOULBROWSER_USE_REAL_CHROME=1 \
soulbrowser chat --execute \
  --max-retries 1 \
  --prompt "打开 https://twitter.com/soulbrowser 并抓取最近的推文列表" \
  --required-schema twitter_feed_v1
```

The planner should emit the canonical step sequence:

1. `navigate` → https://twitter.com/soulbrowser
2. `data.extract-site` (auto)
3. `data.parse.twitter-feed` (references the new parser)
4. `data.deliver.structured` with `schema=twitter_feed_v1`

## 3. Inspect artifacts

Run:

```
soulbrowser artifacts --task <task_id>
```

You should see `structured.twitter_feed_v1.json` plus an optional screenshot if the planner requested one. The artifact matches [`docs/reference/schemas/twitter_feed_v1.json`](../reference/schemas/twitter_feed_v1.json).

## 4. Troubleshooting
-
- If the plan emits an unknown tool name, ensure you're on the latest build (planner prompt enforces the allowlist).
- Twitter occasionally rate-limits headless browsers; re-run with a logged-in Chrome profile if necessary.
- If no tweets are found, the parser falls back to `text_sample`. Validate that `data.extract-site` captured the DOM (check `page.observe_*.json` artifact).

## 5. Next steps
-
- Customize the deliver step to attach screenshots (`data.deliver.structured` payload supports `screenshot_path`).
- Chain additional steps (e.g., `agent.note`) to summarize the feed before delivering the structured artifact.
