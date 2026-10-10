---
type: Decision
title: "Session Trace"
description: "Why the session page gets one Trace view of turns and steps, and how a trace index keeps it fast for sessions with thousands of turns and thousands of steps per turn."
tags:
  - everruns
  - ui
  - sessions
  - events
  - performance
---

# Session Trace

Status: approved 2026-10-10; being built in the order of the delivery plan.

## Abstract

The session page gets a **Trace** tab that replaces Transcript and Timeline with one
debugging view: for every turn, what the agent said (narration, final answer), what it did
(tool calls, batches, approvals, sub-agents, messages), how long each step took (waterfall),
and an inspector with the full detail of the selected step. Events, Approvals, Work, Files
and Cost stay as they are; Events remains the complete raw log and Trace deep-links into it.

The visual reference is the "Session Trace" handoff (prototype plus README) attached to the
project thread. This concept owns the decisions behind it: the data access pattern, the
storage it needs, the API shape and the delivery plan. The prototype owns look and behaviour.

## Problem

Today every session tab reads the raw `events` table through `GET /v1/sessions/{id}/events`
and derives turns and steps in the browser:

- The UI loads the last 200 events, then pages backwards 200 at a time on scroll. A session
  with 200K events needs about 1,000 sequential requests to reach turn 1. Timeline, Events
  and Cost never page at all, so they only see the tail.
- Every new event re-runs several full passes plus a sort over everything loaded, and the
  message list is not virtualized.
- `events` has no turn or tool-call column: `turn_id` lives inside the `context` JSON and is
  not indexed, so "give me turn 2,418" has no cheap query.
- `llm.generation` stores the full prompt of every model call, and `tool.completed` the full
  result. Pulling even one field out of a large JSON value makes PostgreSQL read the whole
  value, so deriving a step list from raw events costs megabytes per turn.
- Paginated reads run a live `COUNT(*)` for `X-Total-Count`, and backward pages "snap" to the
  start of the turn with no bound, so one long turn drags in all of its events.

The design target is a session of **10K turns, 1M events, and single turns with 10K steps**.
The page must render its first useful screen with a constant amount of work, whatever the
session size.

## Decisions

### 1. A trace index, written with the events

Two small tables summarise each session, one row per turn and one row per step. They hold
only what the list needs: kinds, names, short summaries, timings, statuses, counts and the
event sequence range each row came from. Payloads stay in `events` and are read only for the
one step the inspector shows.

| Table | One row per | Key | Holds |
|---|---|---|---|
| trace turns | turn | session, turn number | turn id, sequence range, start/end time, status, prompt preview, step/model/tool/sub-agent/error counts, tokens |
| trace steps | step | session, turn number, step number | kind, sequence range, start time, duration, status, tool name, target and result summaries, tool call id, child session, tokens, narration preview |

- **Turn number** is a per-session ordinal (1, 2, ... 2,418). It is what people read, jump to
  and share in links. It replaces global step numbers, which get too long at this scale.
- **Steps come from small events only**: `turn.*`, `reason.started/completed` (duration,
  usage, text preview), `act.*`, `tool.started/completed`, `output.message.completed`,
  `task.*` and the `llm_generations` row the usage listener already writes. The projector
  never has to read a stored prompt.
- **Summaries are short and bounded**: target from the tool's backend-authored narration
  (see [Tool narration](../execution/tool-narration.md)), result from the first characters
  of the result or the error. Narration and answer previews are capped (about 2 KB); the
  full text is one detail read away.
- **Written right after the events**, by a pure Rust projection
  (`crates/server/src/storage/repositories/session_trace/`). Both event insert paths
  schedule a projection pass once the writing transaction commits, only when a batch holds a
  type the trace reads, so the event write itself pays nothing. Every trace read first catches
  its session up, so a reader always sees every committed event. The projection is a fold:
  events in, row upserts out; deltas never reach it. Doing it in Rust rather than in a
  trigger keeps the rules in one tested function that the backfill and a future journal
  reuse.
- **Rebuildable**: a session's trace rows can be dropped and recomputed from its events. A
  per-session watermark records how far the index has caught up. Existing sessions are
  indexed by a background backfill, newest active first; a session opened before its
  backfill ran is indexed on demand, in bounded batches, and the page shows "indexing older
  turns" until it catches up.
- **Indexes are chosen for the queries below and nothing else**: turn key, turn by start
  sequence (maps a search hit or an event to its turn), turns with errors, step key, step by
  tool call id (pairs `tool.completed` with its `tool.started`), steps with errors.
- Trace rows are deleted with their session. When event retention archives old events, the
  trace rows stay; the inspector then says the raw events were archived.

Rejected: an expression index on `context->>'turn_id'` alone. It finds a turn's events but
still reads every payload to build the step list. Rejected: deriving steps on read with a
cache. The first open of a large session would still read everything. Rejected: the
reporting fact tables. They are org-scoped, filled asynchronously and have no session index.

### 2. Reads are bounded, the browser renders a window

| Read | Returns | Bound |
|---|---|---|
| Trace overview | session totals, minimap buckets (`from, to, steps, duration, errors`), error turn numbers | one row per bucket, about 120 buckets |
| Turns page | turns plus their steps, from the end or around a turn number | about 20 turns and a step budget (about 2,000 rows) per response |
| Turn steps page | a step range of one turn, optionally only errors or one tool | 100 to 500 steps |
| Step detail | facts, input, output, raw events of the step, request summary | payloads over about 50 KB truncated with "load full" |
| Model request | the messages of one model call, by role, paged | 100 messages per page |

- **Repeated calls are batched on read**: five or more consecutive calls of the same tool
  become one batch row with count, failures, p50/p95 and wall time. Successful members are
  paged 100 at a time; failures come first.
- **Long turns are elided on read**: after batching, a turn with more than about 200 visible
  steps returns its first 50 and last 50 plus a gap marker with counts; the turn steps page
  fills the gap on demand, or jumps to its errors.
- **Lifecycle rows** (checkpoints, retries, waits) are read from `events` by sequence range
  for the loaded turns only, when the toggle is on, with repeats collapsed.
- **"New since previous call"**: the step detail compares the message count of this model
  call with the previous one in the same turn and returns only the new messages plus one
  "N earlier messages, unchanged" row. The full request sheet pages through all messages.
  Only these two reads ever touch a stored prompt, and only for one call.
- **Sub-agents** show their summary row only. Expanding one reads the child session's trace
  through the same endpoints, one level deep inline.
- **Search** uses the existing full-text index on events and maps each hit to its turn
  through the trace turns table, then loads that turn.
- **Counts come from the index or from the existing counters**, never from `COUNT(*)` over
  events, matching [session counts](../operations/session-counts.md).

In the browser the loaded turns flatten into one list of turn headers, steps and footers,
rendered with a virtualized list (`@tanstack/react-virtual`, a new dependency), so only the
rows on screen exist in the DOM. Loaded turns are kept in a sliding window; scrolling far
away drops the farthest ones. Nothing derives over the whole session in the browser.

Filters and the selected step live in the URL
(`?view=tools&errors=1&lifecycle=1&turn=2418&step=3`), so a debugging link can be shared.

### 3. Live sessions refresh the tail, not the session

The page keeps the existing SSE stream. When a step-relevant event arrives for the last
turn, the page refetches that turn's steps from the index (debounced), instead of folding
raw events in the browser. The step rules then exist once, in Rust. A running tool already
has a step row from `tool.started`, so it appears with its progress bar right away; deltas
still stream into the visible narration. Auto-scroll happens only when the reader is already
at the bottom.

### 4. No feature flag: added beside the old tabs, then replaces them

Trace is additive, so it ships as a new tab next to Transcript and Timeline with no feature
flag, in line with the rule to keep flags only for what cannot be gated another way. It is not
a capability either: it reads data every session already has. While Trace grows to cover what
Transcript and Timeline show, both stay reachable. Once it covers them, the old routes
redirect: `?tab=transcript` to Trace with `view=messages`, `?tab=timeline` to Trace with
`view=all`, and the old tab code is deleted. That last step waits for the user's OK.

### 5. Answer evidence waits for its own design

Evidence chips (answer claim to step) need a contract from the agent or a post-hoc mapping.
Trace ships without them; the answer renders as prose. This is designed separately.

## Relation to the actor-based design

The proposed actor-based design for durable sessions (not yet in this bundle) makes the events table the session journal and
adds `effect.started/finished` entries. The trace index fits that model directly: it is a
projection of the journal, maintained at append and rebuildable from it. When effect entries
land, the projector maps effect kinds to step kinds and the read API does not change. When
old journal segments move to a bucket, the trace rows stay in PostgreSQL as the index of
cold history, and the inspector fetches the archived segment for detail.

The quadratic size of `llm.generation` (the full prompt stored per model call) is not fixed
here. Storing prompts as references to content-addressed messages belongs with the journal
and blob storage work; the trace design only avoids reading those payloads.

## Success bars

Measured on a seeded local session of 10K turns and 1M events, including one turn with 10K
tool calls and one with nested sub-agents:

- Overview plus the last 20 turns: server p95 under 150 ms, response under 250 KB.
- Any turn by number, including the 10K-step turn: server p95 under 150 ms.
- Step detail: server p95 under 100 ms for payloads under the truncation limit.
- Event insert overhead from the projection: none on the write itself; the pass runs after commit.
- Browser: first trace render under 1 s on a mid-range laptop; scrolling stays at 60 fps with
  every loaded turn expanded; memory stays flat while scrolling the whole session.
- Projection parity: indexing a session incrementally and rebuilding it from its events give
  identical rows (property test over recorded and generated sessions).

## Delivery plan

1. This design.
2. Trace index: migration, the Rust projection on both insert paths, watermark, background
   and on-demand backfill, parity tests, insert benchmark.
3. Trace API: overview, turns page, turn steps page, step detail, model request; batching and
   long-turn elision; OpenAPI; the seeded large-session benchmark against the success bars.
4. Trace tab beside the old tabs: header, control strip, turn rail and minimap, virtualized
   turn list, step rows, inspector, URL state, live tail refresh.
5. Batches and sub-agents expanded inline, model request sheet, lifecycle toggle, search,
   keyboard navigation.
6. Redirects from Transcript and Timeline, old tab code deleted.
