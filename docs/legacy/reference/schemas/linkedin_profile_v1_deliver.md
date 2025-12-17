# LinkedIn Profile delivery example

```json
{
  "id": "parse-linkedin-profile-deliver",
  "tool": "data.deliver.structured",
  "payload": {
    "schema": "linkedin_profile_v1",
    "artifact_label": "structured.linkedin_profile_v1",
    "filename": "linkedin_profile_v1.json",
    "source_step_id": "parse-linkedin-profile",
    "screenshot_path": "artifacts/linkedin-profile-screenshot.png"
  }
}
```

- Replace `source_step_id` with the actual parse step id from your plan (`parse-linkedin-profile` is a suggested default).
- Update `artifact_label` / filename if you need different naming conventions.
- Drop `screenshot_path` when no capture is required (deliver will still attach the JSON artifact).

