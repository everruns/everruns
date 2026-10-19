---
type: Specification
title: Session titles
description: Automatic title admission, model selection, and explicit naming ownership.
tags: [everruns, runtime-resources, models]
---

# Session titles

Automatic naming is an opt-in hosted capability, independent of exposing the
Session tools. It must never delay or fail a normal turn. Explicit titles from
creation, API edits, and tool calls take permanent priority over automatic
maintenance, including same-text writes and in-flight generation races.

The shared organization Small model is an exact provider-bound chat selection,
independent of the main chat and Decisions defaults. An absent selection inherits
the session's normal model chain; an explicit session choice bypasses the Small
model, and a pinned model bypasses both. Changes to the shared default reach
existing sessions. Disabled or unavailable selections do not elect replacements
or spend another account's credentials. Deletion clears the reference.

Sparse local admission avoids a classifier call on every turn. The first
completed user turn is eligible; further attempts require a cooldown and new
user messages. Unchanged titles and failures increase the interval; a changed
title resets it. Reservations persist before paid work so duplicate delivery,
concurrent listeners, restarts, and provider failures do not cause retry storms.
A single bounded request returns keep-or-title. Transcript content is untrusted
data; generation has no tools. Requests use the organization's own provider
credentials, pass budget checks, and emit normal usage accounting.

Automatic writes are fenced against newer attempts, newer results, explicit
renames, archive, and tenant identity. A title and its semantic event commit or
roll back together. The reservation remains committed on failure. Advisory work
may be dropped under load or during shutdown; the next eligible turn can try
again. This deliberately uses bounded process tasks rather than durable queued
jobs: the feature does not promise eventual generation.

The Session capability's legacy `auto_title` prompt asks the main agent to make
an explicit title write. Keep it off when automatic maintenance owns naming;
manual tools remain usable alongside automatic titles.

Sources:

- [Capability configuration](../../crates/capabilities/src/capabilities/automatic_session_titles.rs)
- [Listener and model resolution](../../crates/server/src/listeners/automatic_session_titles/mod.rs)
- [Admission and ownership fences](../../crates/server/src/storage/repositories/sessions/automatic_titles.rs)
- [Behavior tests](../../crates/server/src/listeners/automatic_session_titles/tests.rs)
- [Public documentation](../../docs/capabilities/automatic-session-titles.md)
