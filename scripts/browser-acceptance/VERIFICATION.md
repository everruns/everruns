# Mac mini verification, 2026-11-16

Executed against this foundation on macOS:

- Native authenticated local Chromium acceptance: 2 passed, 0 failed.
- Native authenticated disposable Linux container acceptance: 2 passed, 0 failed.
- Python fixture/proxy unit tests: 3 passed.
- Framework `direct-browser` Engine registration, start and visible-tool inspection: 1 passed.
- Yolop exact-source browser module against this facade: 5 passed, no stubs.

The native cases exercise navigation, accessibility refs, fill/click and resulting
state, screenshot, separate session storage, foreign target rejection, ownership
loss after host restart, browser-origin WebSocket echo, and closing one session
without terminating another. The authenticated proxy rejects missing credentials.
No personal profiles, external accounts or LLM calls were used.

Qualification is incomplete:

- Browserless regression suite: 122 passed, 1 failed in the Mac headless
  `fills_and_submits_a_form_on_a_real_browser` select-all keyboard path. A
  test-only Cmd+A experiment did not fix it and was reverted. Baseline provenance
  is not established; this is an unresolved failure, not a green regression run.
- Full `just pre-push` passed formatting, then was stopped during all-features
  Clippy to preserve 2 GiB free disk. It did not pass.
- Full Yolop build/run is not verified. Published 0.52.0 lacks this API; a local
  0.53.1 family uplift also requires `McpAuthRequest.server_url` migrations.
- Loopback authenticated container transport is not production TLS or
  remote-provider qualification. No personal-browser pairing/handoff was tested.

No release, deployment, push or merge is implied by these results.

## Follow-up qualification

- Clean baseline `b42e96590` reproduced the Mac select-all failure. A narrow
  cross-platform CDP editing-command fix now passes the complete Browserless
  suite: 124 passed, 0 failed. The final form test also verifies initial typing
  before replacement. This supersedes the unresolved regression note above.
- Authenticated WSS local and container acceptance: 2 passed each. TLS uses an
  ephemeral CA trusted only by the child test process. Harness tests: 4 passed,
  including rejection of an untrusted CA and mismatched hostname. No system
  trust changes and no verification bypass. Production remote-provider
  qualification remains outstanding.

- Full native Yolop application test binary now builds against local shared
  source plus the endpoint-identity migration. OAuth tests: 20 passed;
  stored-provider tests: 4 passed; browser tests: 6 passed. Real runtime
  settings activation, navigate/read/screenshot, persisted results and shutdown
  passed on local authenticated Chromium and container authenticated WS/WSS.
  The published-dependency prerequisite still applies; no release was created.
- Personal supervisor: six subprocess tests plus required real upstream
  discovery smoke passed (7 executed, no skips). Official Browse commit
  `c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c`, binary version 0.1.0,
  built locked in a temporary directory. Initialize/tools discovery and exact
  handoff schema passed after requesting the supported plain tool surface.
  Smoke used isolated temporary profiles, autostart disabled and a unique
  loopback port. It did not install/pair an extension or exercise human handoff.

- Independent supervisor review tightened cancelled-response delivery and
  malformed handoff/JSON-RPC envelopes. Final revision: 11/11 tests executed,
  no skips, including required real pinned Browse discovery. No automatic
  replay or successful continuation is inferred from malformed responses.

## Personal-browser durable follow-on (local, unreleased)

The direct-CDP and supervisor results above do not establish personal extension
UX. The follow-on adds authority-owned dispatch claims plus a host coordinator;
its validation is tracked separately:

- Shared command reducer: 28 tests passed for claims, fences, resolution IDs,
  expiry, exact pending-call identity and replay behavior.
- Memory/SQLite adapters: three tests passed, including independent SQLite
  connections racing a claim, reopening, and lost-completion-response replay.
- Cell authority: snapshot/replay test passed.
- Expanded coordinator suite: 13 executed tests passed, covering narrow actions,
  inline images and oversized results, handoff outcomes, policy denial,
  unconditional child cleanup, initialization races, lost replies and foreign
  live claims. The optional real pinned-schema test was explicitly skipped
  pending its executable; it is not counted as an executed passing test.
- Yolop native follow-on: six selected tests passed (configuration, child-process
  opt-in removal, truthful terminal guidance, and durable worker completion),
  plus unconfigured full-runtime startup. This used temporary local source
  overrides; the published dependency policy and manifests were restored.

These are authority/host protocol tests, not actual extension pairing or human
consent tests. No personal profile, account sign-in, release or deployment was
used. The disposable real-extension harness remains a separate qualification
step. Only synthetic test consent may be generated by that harness.
