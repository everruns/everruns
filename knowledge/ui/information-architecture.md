---
type: Decision
title: "Navigation Information Architecture"
description: "How Everruns groups navigation by what you do with a thing, and which alternatives were dismissed."
tags:
  - everruns
  - ui
  - navigation
---

# Navigation Information Architecture

## Abstract

The shell groups navigation by **what you do with a thing**, not by what the thing is.
Five groups carry every destination: Chats, Operational, Building, Registries, Quality,
with Settings pinned below them. The data lives in
[`apps/ui/src/components/layout/sidebar.tsx`](../../apps/ui/src/components/layout/sidebar.tsx)
and renders through the generic section renderer in
[`sidebar-navigation.tsx`](../../apps/ui/src/components/layout/sidebar-navigation.tsx).

This concept exists so that a new entity is placed by a rule instead of by argument.

## The organising principle

A destination belongs to the group that matches the **verb the user brings to it**.
Nothing is grouped by implementation layer, by owning crate, or by how the entity is
stored.

The placement test for a new entity is a single question:

> What does the user do with it — talk to it, look at what it did, author it, register it
> once and reference it by name, or check the quality of something else with it?

The first answer that is true names the group. If two answers feel true, the earlier one
in that list wins; a thing you author and also reference by name is Building, because
authoring is the activity that brings the user to the page.

## What each group asserts

| Group | Assertion |
|---|---|
| **Chats** | Where you talk. First, no section header, and it carries the new-chat affordance. |
| **Operational** | What ran. Recordings and views over them — read, not authored. |
| **Building** | What you author. Editable definitions the user composes and owns. |
| **Registries** | What you register once and reference by name. Mostly-write-once entries other things point at. |
| **Quality** | How you check it. Instruments that judge or observe other entities. |

Settings sits below the groups and is not one of them: it configures the workspace rather
than being a thing the user works on. Durable Execution and Dev keep their existing
policy and dev-mode gating and stay out of the five groups for the same reason.

## The hard cases, worked

* **A Skill is a Registry entry, not Knowledge.** You register a skill once and then
  reference it by name from an agent. You do not sit and author a skill as part of
  building a specific agent, and you do not read it back as a record of what ran.
  Registering-and-referencing beats its surface resemblance to Knowledge indexes.
* **Memory is Building, not Operational.** Memory looks like a recording, but the user
  authors what goes into it and curates it deliberately. The verb is authoring, so it
  sits with the things you compose.
* **Reports is Operational, not Quality.** A report reads what ran. Quality is reserved
  for instruments that judge — Evals and Observers.
* **Identities is Building, not Registries.** An identity is authored per agent
  deployment with credentials and scope decisions, not registered once and forgotten.

## Surface contracts

* **Chats is the unconditional landing route.** It is core functionality, requires no feature
  opt-in, is the first thing in the sidebar, and is the default destination for every user with no
  other intent. A fresh organization can open Chats and start a thread without configuration.
* **A thread is bound to exactly one agent.** Switching agents starts a new thread rather
  than re-pointing an existing one; the thread's transcript is only meaningful against
  the agent that produced it.
* **The nav's thread list is bounded.** Live threads in the nav mean the nav is never the
  same twice, so the Chats entry lists only the few most recently active threads and hands
  the rest to the all-threads page. It also holds its order steady while the pointer is
  inside it, so an arriving turn cannot re-sort a row out from under a click.
* **A session is a read-only recording.** Session detail inspects, it does not edit. Its default
  Transcript preserves the human-readable conversation; Timeline curates how the run executed;
  Events remains the exact emitted ledger. Work, Workspace, and Cost appear when applicable to the
  recording and its enabled capabilities. Watching any of these views stream live is not editing
  the session. Anything that would
  change the session (composing a message, editing a file, writing a secret, steering or
  cancelling a task, enabling a schedule) is absent rather than disabled, including the
  WebMCP tools a browser agent could otherwise reach. The tab set is built by
  `buildSessionNavigation` in
  [`session-header.tsx`](../../apps/ui/src/components/session/session-header.tsx) and
  gated on the session's capability features.
* **Fork is what makes read-only acceptable.** The escape hatches from a recording are
  Fork into chat, Open agent and Export — nothing else. Forking creates a *new* session
  seeded with this one's conversation, workspace and durable storage
  ([forking sessions](../runtime-resources/forking-sessions.md)) and lands the user in it
  as a thread, so a recording can always be turned back into something you can talk to
  without ever mutating the record.
* **Reports is a saved view on Sessions, not a separate page concept.** It shares the
  session model and adds persistence of a query.

## Dismissed options

These were considered and rejected. They are recorded so they are not re-proposed.

* **Agent is the product** — four groups, Compute / Knowledge / Tools / Delivery, framed
  around what an agent is made of. Dismissed because the grouping needed defending on
  every new entity: each addition triggered a fresh argument about which of the four
  substances it was made of, which is exactly the cost the placement rule exists to
  remove.
* **Use / Build modes** — a mode switch that shows either the operating surface or the
  authoring surface. Dismissed because modes hide things, and the builder switches
  between using and building too often for the hidden half to stay out of the way.
  Search does not rescue a hidden item for a user who does not yet know its name.
* **No navigation** — search and in-context links only. Dismissed because it kills
  discovery for the first-time builder, who is the primary user. The idea survives in a
  narrower form: the inspector pattern is kept inside session detail, where the user
  already knows what they are looking at.

## See also

* [Brand Specification](brand.md) — visual language the shell renders in.
* [Documentation Site Specification](documentation.md) — the public documentation surface.
