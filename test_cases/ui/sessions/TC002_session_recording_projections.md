## Description

Verifies that a session recording separates the human-readable Transcript, curated execution
Timeline, and exact raw Events ledger while remaining read-only and responsive.

## Preconditions

- The API, worker, UI, and reverse proxy are running with `AUTH_MODE=none` or an authenticated user.
- A completed session exists with several user/agent turns and model-call events.
- An empty session exists.
- A latency-enabled local test model is available for a live turn.

## Test Data

| Field | Value |
| --- | --- |
| Completed session | At least three turns |
| Empty session | No messages or execution events |
| Desktop viewport | 1280 × 800 or wider |
| Narrow viewport | 390 × 844 |

## Steps

1. Open `/sessions/<completed-session-id>` and inspect the resulting URL, title, tabs, and active tab.
2. Inspect Transcript, then start a latency-enabled turn and observe output while it streams and
   after it completes.
3. Confirm Transcript contains conversation messages and compact completed work narration but no
   composer, send, edit, cancel, or other mutation control.
4. Open Timeline and inspect the same live and completed turn.
5. Expand one model or tool detail and inspect its worker correlation or payload.
6. Open Events and inspect one raw event payload.
7. Use browser Back and Forward across Transcript, Timeline, and Events.
8. Open the legacy `/sessions/<completed-session-id>/chat` URL.
9. Repeat Transcript and Timeline inspection at the narrow viewport.
10. Open Transcript and Timeline for the empty session.
11. Open an unknown session ID and observe the error state; reload a known session and observe the
    loading skeleton before data settles.

## Expected Result

- The base and legacy chat routes redirect to `/sessions/<id>/transcript`.
- Navigation order is Transcript, Timeline, optional Work, Events, optional Workspace, Cost.
- Transcript is active by default, has the Transcript page title, fills the recording content area,
  replays completed conversation, and appends live output without a composer or mutation controls.
- Timeline fills the recording content area without a transcript rail, streams/replays curated
  execution steps, and keeps raw worker IDs and payloads collapsed behind details.
- Events shows event sequence, type, timestamp/metadata, and the complete raw payload rather than
  either curated projection.
- Back/Forward restores the correct route and active tab.
- Empty, loading, and missing-session states are clear and do not expose mutation controls.
- Desktop and narrow layouts remain readable without side-by-side independent transcript/timeline
  scrollers.
