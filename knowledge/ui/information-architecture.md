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
Five groups carry every destination: Chats, Operational, Building, Registers, Quality,
with Settings pinned below them. The data lives in
[`navigation.ts`](../../apps/ui/src/lib/navigation.ts)
and renders through the generic section renderer in
[`sidebar-navigation.tsx`](../../apps/ui/src/components/layout/sidebar-navigation.tsx).

This concept exists so that a new entity is placed by a rule instead of by argument.

## Command search stays aligned

Command-K is another view of the current navigation, including side-chat shortcuts and
Settings. Page labels, icons, groups, aliases, and availability come from the shared
[navigation model](../../apps/ui/src/lib/navigation.ts) and
[Settings model](../../apps/ui/src/lib/settings-navigation.ts); feature work extends the
owning model instead of maintaining a separate palette list. Collapsed destinations remain
searchable, while development-only, disabled, and unavailable destinations stay hidden.
Opening the palette shows available destinations in layout order without fetching entity
lists. Typing searches page names, group context, aliases, and existing entity sources.
Static pages are not truncated by the per-entity result cap: a broad Settings or group query
must still expose every matching destination. [Regression tests](../../apps/ui/src/__tests__/use-global-search.test.tsx)
compare search against the live models so newly registered destinations inherit coverage.

## The organising principle

A destination belongs to the group that matches the **verb the user brings to it**.
Nothing is grouped by implementation layer, by owning crate, or by how the entity is
stored.

The placement test for a new entity is a single question:

> What does the user do with it, talk to it, look at what it did, author it, register it
> once and reference it by name, or check the quality of something else with it?

The first answer that is true names the group. If two answers feel true, the earlier one
in that list wins; a thing you author and also reference by name is Building, because
authoring is the activity that brings the user to the page.

## What each group asserts

| Group | Assertion |
|---|---|
| **Chats** | Where you talk. First, no section header, and it carries the new-chat affordance. |
| **Operational** | Recordings and external exposure: Sessions and Exposures. |
| **Building** | Agents, Playground, Harnesses, Sandboxes, and Virtual Users, in that order. |
| **Registers** | Reusable entries and curated resources, including Sandbox Templates, Knowledge indexes, and Memory. |
| **Quality** | Evaluation, observation, and reporting. |

Settings sits below the groups and is not one of them: it configures the workspace rather
than being a thing the user works on. Durable Execution and Dev keep their existing
policy and dev-mode gating and stay out of the five groups for the same reason.

## The hard cases, worked

* **A Skill is a Registry entry, not Knowledge.** You register a skill once and then
  reference it by name from an agent. You do not sit and author a skill as part of
  building a specific agent, and you do not read it back as a record of what ran.
  Registering-and-referencing beats its surface resemblance to Knowledge indexes.
* **Memory and Knowledge indexes are Registers.** They are curated resources referenced
  by agents; keeping them with the other reusable entries leaves Building focused on
  composing and trying agents.
* **Sandboxes is Building; Sandbox Templates is Registers.** Live execution environments
  sit beside agents and harnesses, while their reusable definitions stay in Registers.
* **Reports is Quality.** Reporting sits with Evals and Observers, leaving Operational
  focused on recordings and external exposure. Approvals stays within its session.
* **Identities is Building, not Registers.** An identity is authored per agent
  deployment with credentials and scope decisions, not registered once and forgotten.

## Playground

Playground belongs under Building: it is the team's place to try the agents they are authoring.
Chats remains the personal conversation surface and the unconditional landing route. Playground
chats are ordinary organisation-scoped sessions, listed independently of personal Chats.
They use the same header, transcript, composer, streaming context, and workspace viewer.

A conversation fixes its counterpart and end-user virtual user at creation. The default subject is
the operator's linked virtual user; selecting another subject requires organisation admin authority.
Every organisation member may inspect shared conversations. Sending as another subject rechecks
that authority on each message. The selected identity and human operator remain separate audit facts;
simulation never supplies management authority or private end-user connection grants. Private user
memory is excluded from these shared workspaces, including delegated runs. Explicit agent service
connections retain their existing access rules. The personal Platform Chat harness is unavailable.

The initial experience starts with a fresh workspace. Files is a secondary inspection view,
not a prerequisite for starting a conversation. Realtime voice remains on the personal Chat surface
until its direct transcript path supports fixed test-subject attribution. A persistent Open session action leads to the same
recording and its timeline. Archiving is shared. Pinning stays on personal Chats. The library offers
server-paginated search, an agent filter, grouping by day, agent, or none, and active/archived
views.

Playground is a standard surface, available without deployment or organisation feature flags.
Its list, setup, and detail pages use the shared page layout: breadcrumbs, masthead, control strip,
and context rail. Each list row opens its chat. Agent and virtual-user chips link to their detail
pages, and the row shows who started the chat. The UI consistently calls these Playground chats.
See [source and binding policy](../../crates/server/src/domains/sessions/playground.rs) and
[shared creation flow](../../apps/ui/src/components/chat/new-chat-form.tsx).

## Surface contracts

### Adopted Chat workspace

The Chat threads adoption feature is disabled by default. An organisation owner/admin
opts in through Features settings; deployment availability alone is not enrolment.
Without that opt-in, the existing sidebar and conversation routes retain their behavior.

In the adopted workspace the global sidebar contains only Chat. The permanent conversation
stays alongside an integrated Threads panel containing personal side conversations and
background work owned by that permanent conversation. Creation, search, history, and
resolution live in the panel. Opening a thread preserves the permanent conversation;
the panel can expand and becomes a full-screen drawer on mobile. Existing URLs remain valid.

An idle conversation remains open. Explicit Resolve/Reopen reuse the durable archive
lifecycle; resolved conversations retain their transcript and require reopening to send.
Background work retains its existing task lifecycle and input/cancellation controls.
Failed work needs attention rather than disappearing as resolved. Work cards link directly
into the panel. Agent, harness, owner, and Playground boundaries remain unchanged.

Implementation: [workspace](../../apps/ui/src/components/chat/chat-workspace.tsx),
[rollout catalog](../../crates/server/src/records/feature_flags.rs), and
[lifecycle grouping](../../apps/ui/src/lib/chat-thread-work.ts).

### Default Chat

* **Chat is the unconditional landing route.** It directly opens the user's permanent
  Platform Chat conversation. The permanent conversation cannot be renamed, unpinned,
  archived, deleted, or reassigned. Server resolution and database uniqueness arbitrate
  concurrent tabs and retries; an existing conversation is always adopted.
  Its sidebar entry remains a prominent navigation control on every route;
  **New side chat** is the subordinate action for a separate conversation.
* **Chat has one managed Agent on Generic.** The Agent owns identity, platform access,
  instructions, introduction, and starters. No Agent or harness picker appears in Chat.
  New chat opens an empty draft and creates a fresh side conversation on first send,
  with the same Agent and owner. Durable private/shared memory remains available;
  conversation history and workspace files are not copied.
* **Chat history lists side conversations only.** Source, managed Agent, owner, and the
  permanent-conversation exclusion are applied before pagination. Playground conversations
  and arbitrary Agent runs remain in their respective surfaces. Existing recording URLs
  remain inspectable through Sessions.
* **The nav's thread list is bounded.** Live threads in the nav mean the nav is never the
  same twice, so the Chat entry lists only the few most recently active threads and hands
  the rest to the all-threads page. It also holds its order steady while the pointer is
  inside it, so an arriving turn cannot re-sort a row out from under a click.
* **Archiving is how a thread leaves the list without leaving existence.** Threads accumulate,
  and the bounded nav list makes that cost visible first. Archiving drops a thread out of the
  nav and out of the all-threads page while keeping its transcript and its URL; the all-threads
  page always offers the filter that brings archived threads back, because it is the only route
  to one short of its URL. Deletion stays a separate, destructive action. Unlike pinning, which
  is a personal shortcut, archiving is a statement about the thread and applies to everyone who
  can see it.
* **A session is a read-only recording.** Session detail inspects, it does not edit. Its default
  Transcript preserves the human-readable conversation; Timeline curates how the run executed;
  Events remains the exact emitted ledger. Approvals is always present: each card pairs a request
  with the grant recorded for it, so an admin sees what was asked and what was written down as
  approved, including who approved it. Work, Files, and Cost appear when applicable to the
  recording and its enabled capabilities. Watching any of these views stream live is not editing
  the session. Anything that would
  change the session (composing a message, editing a file, writing a secret, steering or
  cancelling a task, enabling a schedule) is absent rather than disabled, including the
  WebMCP tools a browser agent could otherwise reach. The tab set is built by
  `buildSessionNavigation` in
  [`session-header.tsx`](../../apps/ui/src/components/session/session-header.tsx) and
  gated on the session's capability features.
* **The tab bar is a map of the recording, so tabs carry counts.** Work, Events and
  Files are badged with what is behind them; Timeline (the whole run), Approvals and Cost (a
  single figure already shown in the header) are not. Approvals has no denormalized counter, and
  the tab does not scan the event log just to paint a number. An empty tab renders with no badge
  rather than a `0`, so absence reads as absence. The counts ride on the session payload
  the page already fetches — never a per-tab request — and are served from denormalized
  counters maintained by trigger (`sessions.event_count`, `sessions.task_count`,
  `workspaces.file_count`), so opening a session never scans `events`. They are a
  snapshot at load; a live session's badges refresh when the session query does. A count
  the server cannot produce cheaply is omitted, and an absent badge is the honest
  answer — see [session counts](../operations/session-counts.md).
* **Fork is what makes read-only acceptable.** The recording actions are Fork into chat
  for personal Platform Chat, Test in Playground for other agents, and Export. The agent
  name in the header links to its detail page. Forking creates a *new* session
  seeded with this one's conversation, workspace and durable storage
  ([forking sessions](../runtime-resources/forking-sessions.md)) and lands the user in it
  as a thread, so a recording can always be turned back into something you can talk to
  without ever mutating the record.
* **Reports is a saved view on Sessions, not a separate page concept.** It shares the
  session model and adds persistence of a query.
* **The breadcrumb names the owning group, and derives it from the nav table.** The
  sidebar teaches the five-group model to people who navigate; the breadcrumb teaches it
  to everyone arriving by a direct link or saved view. `PageBreadcrumb` reads the group from the
  section definitions in [`navigation.ts`](../../apps/ui/src/lib/navigation.ts) rather than
  taking it as a prop, so a page's group is never stated in two places that can disagree
  and moving a route between groups needs no page-level change. The group is a label, not a
  link, there is no group index page to point at, and inventing one would add a hop nobody
  wants. Pages outside a labelled section (Chats, Settings) take no prefix.

## Dismissed options

These were considered and rejected. They are recorded so they are not re-proposed.

* **Agent is the product**: four groups, Compute / Knowledge / Tools / Delivery, framed
  around what an agent is made of. Dismissed because the grouping needed defending on
  every new entity: each addition triggered a fresh argument about which of the four
  substances it was made of, which is exactly the cost the placement rule exists to
  remove.
* **Use / Build modes**: a mode switch that shows either the operating surface or the
  authoring surface. Dismissed because modes hide things, and the builder switches
  between using and building too often for the hidden half to stay out of the way.
  Search does not rescue a hidden item for a user who does not yet know its name.
* **An organisation-wide Approvals destination.** A sidebar list of every request and every
  grant, as separate rows, does not say what was approved: the model phrases the ask and the
  grant differently, and the two rows do not sit together. Dismissed. The pair is read on the
  session it belongs to.
* **No navigation**: search and in-context links only. Dismissed because it kills
  discovery for the first-time builder, who is the primary user. The idea survives in a
  narrower form: the inspector pattern is kept inside session detail, where the user
  already knows what they are looking at.

## See also

* [Brand Specification](brand.md), visual language the shell renders in.
* [Documentation Site Specification](documentation.md), the public documentation surface.
