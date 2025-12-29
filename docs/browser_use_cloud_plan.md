## Browser Use Cloud Inspired Session & Live Viewer Plan

### Background

- Browser Use Cloud exposes *Sessions* that persist browser state, provide `liveUrl` viewing links, and can be shared publicly. (docs.cloud.browser-use.com/concepts/session lines 11-66)
- Tasks run inside sessions and can be created implicitly (auto session) or explicitly (custom session) (lines 35-45).
- They also offer direct CDP browser sessions, stealth infra, profiles, file sync, and skills/workflows as reusable recipes.

### Goals for SoulBrowser

1. Provide a first-class Session/Profile service with persistent state.
2. Offer live viewing & shareable URLs similar to Browser Use Cloud.
3. Allow AI tasks to attach to sessions and stream logs/results in real time.
4. Expose direct CDP access for advanced users.
5. Upgrade SDK/CLI to mirror the new workflows.

### Implementation Plan

#### 1. Session & Profile Service
- Define `Profile` objects (cookies, storage, auth artifacts) stored per tenant.
- Extend Session metadata (`profile_id`, `status`, timestamps, task history) in `soulbrowser-kernel`.
- Add REST/gRPC endpoints to create/list/stop sessions and bind profiles.

#### 2. Live Viewer & Sharing
- Build a websocket streamer emitting screenshots + logs from the scheduler.
- Add `session.live_url` and optional `public_share_token` with signed access.
- Update `web-console` to list sessions and embed a live viewer.

#### 3. Task Orchestration
- Let `task.create` accept `session_id`/`profile_id` for auto vs custom session flows.
- Provide `task.logs` streaming endpoints and UI cards for execution status.

#### 4. Direct Control / CDP Bridge
- Expose authenticated CDP endpoints per session (reusing `l0_bridge`).
- Document how to `chromium.connectOverCDP` with a SoulBrowser session.

#### 5. Infra & Files (later phases)
- Centralize stealth/proxy config in profiles.
- Automate download storage & retrieval via session-scoped file APIs.

#### 6. SDK/CLI Updates
- Extend TypeScript/Python SDKs with `createSession`, `watchLive`, `shareSession` helpers.
- CLI commands: `soulbrowser session create`, `session live`, `session share`.

#### 7. Security
- Require auth tokens for live URLs; issue short-lived signed links.
- Rate-limit session creation/streaming per tenant.

### Roadmap
1. Finalize API schemas & update architecture docs.
2. Implement backend session/profile storage & task wiring.
3. Ship live streaming hub and console viewer.
4. Publish SDK/CLI updates + CDP bridge documentation.
5. Add infra/file enhancements and run integration tests.
6. Launch updated quickstarts demonstrating live viewer + session reuse.

### Current Progress

- ✅ Added a kernel-level `SessionService` that persists session metadata, tracks live overlays/frames, and taps into the task stream so every screenshot artifact feeds real-time viewers.
- ✅ Surfaced `/api/sessions` REST endpoints (list/detail/create/share) plus an SSE stream at `/api/sessions/{id}/live` with share-token support so the web console and external viewers can watch automation in real time.
- ✅ Shipped a new **会话** section in the React console that lists persistent sessions, spins up live viewers with screenshot playback, and lets operators mint or revoke share links that mirror Browser Use Cloud's `liveUrl` experience.
