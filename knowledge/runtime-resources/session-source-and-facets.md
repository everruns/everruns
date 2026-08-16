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

## Index shape

The list and every aggregate share one predicate shape, org, then some subset
of source, agent, owner, and a creation window, ordered by `created_at` or
`updated_at`. Migration 118 extends the existing `(org_id, created_at DESC)`
index (EVE-697) so each added filter still begins from an org-scoped index
scan. Org scoping is part of the predicate itself, not a post-filter: a count
must never span organizations.
