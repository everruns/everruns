---
type: Specification
title: "Session Source and List Facets"
description: "How a session records the way it was started, and how the sessions list filters and counts over it."
tags:
  - everruns
  - runtime-resources
---
# Session Source and List Facets

Every session records **how it was started**, and the sessions list serves
**filters and counts over that same predicate**. Together these make the
sessions surface answer two questions it previously could not: *where did this
run come from* and *how many runs match what I am looking at*.

## Motivation

The sessions list filtered by agent and title substring only. That is enough to
find a session you already know about and nothing else. An operator asking
"which runs did a schedule start last night", or "show me my chat threads", or
"how many of these failed", had to fetch every row and reduce client-side,
which does not survive the row counts a busy org produces.

Two things were missing, and they are related: a dimension to group by, and
aggregates that do not require reading the page.

## Source

`Session.source` is a **closed, typed set**, not a free-form string, because
the facet rail enumerates it. See
[`SessionSource`](../../crates/core/src/session.rs) for the variants.

Three rules govern it:

* **Server-owned.** Each ingress path sets the value at creation. A client may
  declare only `chat` or `api` on `POST /v1/sessions`; anything else is
  rejected. A facet that says "started by a schedule" is worthless if a caller
  can claim it.
* **Structurally derived where possible.** A session created under a
  `parent_session_id` is `subagent` regardless of what the caller asked for,
  and a fork inherits its parent's source. Rules that read the row beat rules
  that trust the request.
* **Explicitly unknown, never guessed.** The backfill in migration 118 infers
  from the app's channel type, the subagent parent pointer, and the two tag
  conventions the server itself writes. Rows it cannot place stay `unknown`
  rather than being folded into a real facet and quietly inflating it.

### A chat thread is an ordinary session

There is no chat API. A chat thread is a session created via
`POST /v1/sessions` with an agent and `source: "chat"`; the Chats sidebar is
this list endpoint with `source=chat`, `mine=true`, `order=last_activity`.
Making threads findable is a filter, not a surface.

## Activity: the status the list shows

`SessionStatus` describes execution state (`started`, `active`, `idle`,
`waiting_for_tool_results`, `paused`) and has **no notion of failure**: a
session whose last turn errored is simply `idle`. The list and the masthead
both need failure, so `SessionActivity` derives an outcome-oriented value from
the execution status plus the outcome of the most recent terminal turn.

`sessions.last_turn_status` carries that outcome, maintained by statement-level
triggers on `events`, the same incremental-counter pattern
`turn_count`/`tool_call_count` established, for the same reason: the list must
not rescan event history per row.

The derivation exists twice, in Rust
([`SessionActivity::derive`](../../crates/core/src/session.rs)) and in SQL
([`ACTIVITY_SQL`](../../crates/server/src/storage/repositories/sessions.rs)),
because the list filters in the database and the in-memory backend filters in
Rust. They are pinned to one truth table by test; change them together.

## Run summary: the sentence the header shows

`SessionActivity` says *whether* a run failed. `sessions.run_summary` says
**what happened** — one generated sentence naming what the run did and, when it
failed, which step failed and why (EVE-867). The session detail header renders
it in place of the start timestamp, and it is the string the sessions list
failure column and failure notifications should read rather than each deriving
their own wording.

Three properties are load-bearing:

- **It is generated once, not derived per read.** A read-time derivation would
  put a paid LLM call on every page load, and give three surfaces three
  wordings for one run.
- **Absence is normal.** Chat threads get none — a conversation has a title and
  a transcript, not a verdict — and deployments with no utility LLM
  (`DisabledUtilityLlmService`, the OSS default) never write it. Every reader
  needs a fallback.
- **It never gates a run.** Generation is spawned from the terminal-turn event
  and awaited by nobody, so a slow or failing summariser costs a nicer header
  and nothing else. It is also charged to the org-independent utility LLM, so
  the Cost tab keeps reporting what the *run* spent.

Unlike `last_turn_*`, no trigger maintains it: it cannot be derived from
`events` in SQL. Because generation is out of band, a slow call for turn N can
land after turn N+1 was summarised, so writes are fenced on
`run_summary_turn_sequence` in the `WHERE` clause — the same
never-move-backwards guard the `last_turn_sequence` trigger applies. See
[`RunSummaryService`](../../crates/server/src/services/run_summary.rs).

The transcript is untrusted input to the summariser. The model receives a
bounded, delimited digest of turn structure and failures, labelled as data, so a
session whose transcript contains instructions cannot steer its own summary.

## Counts

`GET /v1/sessions/facets` serves the facet rail and the masthead as aggregates
over the *same* filter predicate as `GET /v1/sessions`. Two decisions are worth
recording:

**Each facet excludes its own selection.** A dimension is counted with every
other filter applied but its own dropped. Counting `by_source` under an active
`source=chat` filter would report `chat: N` and zero for everything else,
making the rail a dead end after the first click. Excluding the dimension's own
selection is what makes multi-select work.

**Counts come from `sessions`, not `fact_session`.** The reporting projection
is the natural place to aggregate and was the first choice, but it is an
eventually-consistent mirror of the same row that carries neither `source` nor
the last-turn outcome. It could not answer these filters, and where it could it
would report a count that disagrees with the page rendered beside it. Facets
belong to the list, so they read what the list reads. `fact_session` remains
the right source for analytics that tolerate projection lag.

## Archive: the "put it away" bit

`sessions.archived_at` (migration 124) records that a session was deliberately
set aside. Default list results and every facet count exclude archived rows;
`include_archived=true` widens both together, so the counts still describe the
page. `PUT`/`DELETE /v1/sessions/{id}/archive` set and clear it, idempotently:
re-archiving keeps the original timestamp, so it records when the session was
*first* put away.

Three distinctions are the point:

* **Not status.** `SessionStatus`/`SessionActivity` describe execution. An
  archived session can be idle, completed, or failed; archiving says nothing
  about how the run went.
* **Not deletion.** The events stay. A chat thread is the record of what an
  agent did, so the answer to "I'm done with this" must not be destruction. The
  thread keeps its URL and opens normally.
* **Not per-viewer.** Unlike `pinned_sessions`, archive lives on the session
  row, so a thread archived by its owner is archived for everyone who can see
  it. Pinning is a personal shortcut; archiving is a statement about the thread.

The chat surface is where this is spent: `/chats` hides archived threads and
offers a "Show archived" filter to bring them back, and that filter is always
present — it is the only way back to an archived thread short of its URL.

## Index shape

The list and every aggregate share one predicate shape, org, then some subset
of source, agent, owner, and a creation window, ordered by `created_at` or
`updated_at`. Migration 118 extends the existing `(org_id, created_at DESC)`
index (EVE-697) so each added filter still begins from an org-scoped index
scan. Org scoping is part of the predicate itself, not a post-filter: a count
must never span organizations. Migration 124 adds a partial
`(org_id, updated_at DESC) WHERE archived_at IS NULL` index for the default
"active threads, most recent first" shape, so archiving shrinks the hot path
instead of adding to it.
