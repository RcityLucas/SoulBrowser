# HackerNews Feed delivery example

```json
{
  "id": "parse-hackernews-feed-deliver",
  "tool": "data.deliver.structured",
  "payload": {
    "schema": "hackernews_feed_v1",
    "artifact_label": "structured.hackernews_feed_v1",
    "filename": "hackernews_feed_v1.json",
    "source_step_id": "parse-hackernews-feed",
    "screenshot_path": "artifacts/hackernews-feed-screenshot.png"
  }
}
```

- Replace `source_step_id` with the actual parse step id from your plan (`parse-hackernews-feed` is a suggested default).
- Update `artifact_label` / filename if you need different naming conventions.
- Drop `screenshot_path` when no capture is required (deliver will still attach the JSON artifact).

