---
type: Decision
title: "Org Home"
description: "Why the organization has one Home page that ranks what needs a person before showing how the agents ran, and why Chat stays the landing route with Home beside it in the Chat workspace."
tags:
  - everruns
  - ui
  - navigation
  - health
---

# Org Home

## Abstract

Home answers two questions in order: what in this organization needs a person now, and how
its agents ran in the last 24 hours. It is a page above Chat in the sidebar, not the landing
route. Source: [`components/home/`](../../apps/ui/src/components/home/), the ranking rules in
[`lib/org-home.ts`](../../apps/ui/src/lib/org-home.ts), and the arrival logic in
[`chat-mode/landing.ts`](../../apps/ui/src/components/chat-mode/landing.ts).

## Decisions

- **Chat stays the landing route** ([Navigation](information-architecture.md)). Home is one
  click away. With the Chat workspace on ([Chat Mode](chat-mode.md)), arrival opens that same
  Chat beside Home instead of full width, so the first page Chat can read is the attention list.
  `/` redirects to `/chats?landing`; the client picks the target once organization flags have
  loaded, because the server redirect cannot see them. `?chat=open` on any app page opens the
  page-side Chat and is then dropped from the URL. `/dashboard` redirects to Home.
- **Home merges, it does not detect.** Every signal comes from the page that owns it, through the
  endpoint that page already uses: agent setup problems from the Agents page model
  ([Agents Home](agents-home.md)), open health issues (channel, organization limit and
  background-work detectors), sessions paused at their budget limit, sandboxes with an attention
  reason, and system doctor checks for viewers allowed to see System. The one rule Home adds is
  the failure burst: an agent whose last two hourly buckets hold at least three turns, half or more
  of them failed. Home composes these on the client; there is no Home endpoint to keep in step.
- **Unlike the Agents page, attention here includes runtime.** The Agents page lists only
  configuration a builder fixes. Home is where an operator looks first, so a failing agent, a
  paused run and the organization's turn limit belong in the same list.
- **One ranked list, each row with one way out.** Blocking items first, then the condition that
  started earliest. Each row names its source and subject and links to the page where it is
  fixed. Snoozing stays on that page; a snoozed health issue leaves Home too. A channel issue the
  Agents model already reported is not repeated from health issues. When nothing is wrong the
  list collapses to one line.
- **Charts are Monitoring's.** With Monitoring enabled, Activity is the compact form of Monitoring
  → Timeline: the same `TimeSeriesChart`, series, colors and 5-minute buckets from
  `GET /v1/monitoring/timeline?range=24h`, whose current and previous summaries also feed the turn
  and failure-rate comparisons. Without Monitoring, the agents activity buckets draw turns per
  hour. Home never designs its own chart.
- **No spend.** Model spend stays in Monitoring and Reports. Home carries no billing framing.
- **A new organization gets steps, not zeros.** Until it has an agent of its own, the key numbers
  and agent sections are replaced by a three-step Get started list.
- **Chat reads what Home shows.** The attention rows are registered as page rows, so Chat beside
  Home answers "what should I look at first" from the same ranking.
- **Layout follows the pane.** Home is a container (`@container/home`), so its grids respond to the
  page pane in the Chat workspace rather than to the browser viewport.

## Not yet

Observer findings and the latest eval scores are not on Home: there is no organization-wide
endpoint for them, and Evals is being rebuilt. Add them as attention items and a Quality card
once that lands.
