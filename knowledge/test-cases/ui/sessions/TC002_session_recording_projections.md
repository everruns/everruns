---
type: Test Case
title: "TC002: Sessions - Session Recording Projections"
description: "Verifies that a session recording opens on Trace, that old Transcript and Timeline links land there, and that Trace and the raw Events ledger stay read-only and responsive."
tags:
  - everruns
  - test-case
  - ui
  - sessions
---
# TC002: Sessions - Session Recording Projections

## Description

Verifies that a session recording opens on the Trace view (see
[session-trace](../../../ui/session-trace.md)), that old Transcript and Timeline links land there,
and that Trace and the exact raw Events ledger stay read-only and responsive.

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
2. Open `/sessions/<completed-session-id>/transcript`, `/timeline` and `/chat`.
3. On Trace, start a latency-enabled turn and observe it while it runs and after it completes.
4. Confirm Trace shows turns, steps and the inspector but no composer, send, edit, cancel, or
   other mutation control.
5. Select a model or tool step and inspect its input, output and raw events.
6. Open Events and inspect one raw event payload.
7. Use browser Back and Forward across Trace and Events.
8. Repeat the Trace inspection at the narrow viewport.
9. Open Trace for the empty session.
10. Open an unknown session ID and observe the error state; reload a known session and observe the
    loading skeleton before data settles.

## Expected Result

- The base, `/transcript`, `/timeline` and legacy `/chat` routes redirect to
  `/sessions/<id>/trace`.
- Navigation order is Trace, Approvals, optional Work, Events, optional Files, Cost; there is no
  Transcript or Timeline tab.
- Trace is active by default, has the Trace page title, refreshes its last turn while the session
  runs, and has no mutation controls.
- Events shows event sequence, type, timestamp/metadata, and the complete raw payload.
- Back/Forward restores the correct route and active tab.
- Empty, loading, and missing-session states are clear and do not expose mutation controls.
- At the narrow viewport the step list fills the width and the inspector opens over it.
