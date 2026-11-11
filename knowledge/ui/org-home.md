---
type: Decision
title: "Org Overview"
description: "Why the organization has one Overview page that ranks what needs a person before showing how the agents ran, why its sidebar entry counts those items, and why arrival opens Main chat beside it."
tags:
  - everruns
  - ui
  - navigation
  - health
---

# Org Overview

## Abstract

Overview (route `/home`, called Home until 2026-11-10) answers two questions in order: what
in this organization needs a person now, and how its agents ran in the last 24 hours. It is
the first entry of the Building section, not the landing route. Its sidebar entry shows how
many items wait on the attention list, and nothing when there are none; the count reuses the
same queries and ranking ([`use-home-attention.ts`](../../apps/ui/src/components/home/use-home-attention.ts)),
so it can never disagree with the page. Source: [`components/home/`](../../apps/ui/src/components/home/), the ranking rules in
[`lib/org-home.ts`](../../apps/ui/src/lib/org-home.ts), and the arrival logic in
[`chat-mode/landing.ts`](../../apps/ui/src/components/chat-mode/landing.ts).

## Decisions

- **Arrival lands on Overview beside Main chat** ([Chat](chat-mode.md)) in the default Chat
  first layout, so the first page Chat can read is the attention list. Classic lands on
  full-screen Main chat. `/` redirects to `/chats?landing`; the client redirects once the
  organization and the layout preference have loaded. `/dashboard` redirects to Overview.
- **Overview merges, it does not detect.** Every signal comes from the page that owns it, through the
  endpoint that page already uses: agent setup problems from the Agents page model
  ([Agents Home](agents-home.md)), open health issues (channel, organization limit and
  background-work detectors), sessions paused at their budget limit, sandboxes with an attention
  reason, and system doctor checks for viewers allowed to see System. The one rule Overview adds is
  the failure burst: an agent whose last two hourly buckets hold at least three turns, half or more
  of them failed. Overview composes these on the client; there is no Overview endpoint to keep in step.
- **Unlike the Agents page, attention here includes runtime.** The Agents page lists only
  configuration a builder fixes. Overview is where an operator looks first, so a failing agent, a
  paused run and the organization's turn limit belong in the same list.
- **One ranked list, each row with one way out.** Blocking items first, then the condition that
  started earliest. Each row names its source and subject and links to the page where it is
  fixed. Snoozing stays on that page; a snoozed health issue leaves Overview too. A channel issue the
  Agents model already reported is not repeated from health issues. When nothing is wrong the
  list collapses to one line.
- **Charts are Monitoring's.** With Monitoring enabled, Activity is the compact form of Monitoring
  → Timeline: the same `TimeSeriesChart`, series, colors and 5-minute buckets from
  `GET /v1/monitoring/timeline?range=24h`, whose current and previous summaries also feed the turn
  and failure-rate comparisons. Without Monitoring, the agents activity buckets draw turns per
  hour. Overview never designs its own chart.
- **No spend.** Model spend stays in Monitoring and Reports. Overview carries no billing framing.
- **A new organization gets steps, not zeros.** Until it has an agent of its own, the key numbers
  and agent sections are replaced by a three-step Get started list.
- **Chat reads what Overview shows.** The attention rows are registered as page rows, so Chat beside
  Overview answers "what should I look at first" from the same ranking.
- **Layout follows the pane.** Overview is a container (`@container/home`), so its grids respond to the
  page pane in the Chat workspace rather than to the browser viewport.

## Not yet

Observer findings and the latest eval scores are not on Overview: there is no organization-wide
endpoint for them, and Evals is being rebuilt. Add them as attention items and a Quality card
once that lands.
