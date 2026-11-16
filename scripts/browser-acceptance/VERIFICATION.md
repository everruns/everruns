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
