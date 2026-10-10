---
type: Decision
title: "Agents Home"
description: "Why the Agents page shows what each agent is doing, how it is reached and what needs attention, and why channels replace the Exposures page there."
tags:
  - everruns
  - ui
  - agents
  - channels
---

# Agents Home

## Abstract

The Agents page is not a registry of
definitions; it is the place where a team runs its agents. Each row answers three
questions at a glance: what the agent is doing now, how it is reached, and how the last 24
hours went. A second view on the same page lists every channel in the organization and
replaced the Exposures page. Definitions (prompt, model, harness) stay on the
[Agent Page](agent-page.md). Source: `apps/ui/src/components/agents/home/`, the pure rules in
`apps/ui/src/lib/agents-home.ts`, and `GET /v1/agents/activity`
(`crates/server/src/api/agent_activity.rs`).

## Decisions

- **Company agents, not personal assistants.** The page has no approval prompts or "waiting on
  you" states. Agents here run unattended for the organization.
- **Attention means configuration.** Needs attention lists only problems a builder fixes in the
  agent's setup: a Slack permission or connection the health checker found, a default model that
  is no longer enabled, and a channel live to anyone without sign-in. A failed run or the org's
  active-turn limit is runtime noise and stays out. The list is one collapsed line, hidden when
  nothing is wrong.
- **A run is a turn.** Run bars and failure counts read `turn.started` and `turn.failed`, the same
  events the session counters and the active-turn limit use. "Active turns" is sessions with
  `status = 'active'`, so it matches the limit it is shown against; "Agents running" counts the
  agents behind them.
- **One read for the activity.** `GET /v1/agents/activity` returns every agent's load, 24 hourly
  buckets, active triggers, and every channel's 7 daily session buckets, so the busiest orgs do not
  get an N+1 page. Channel configuration still comes from each agent's channel list: Apps are
  archival and no longer hold an agent's channels, so the old Exposures list read nothing. It reads the window through sessions touched in it and their turn events, so
  cost follows recent activity, not history.
- **Channels are first class, and the word "Exposures" is retired on this page.** A channel is
  Live, Draft, Paused (turned off on the channel or on its agent), or Agent archived. State is
  resolved the same way as the Exposures view (see [Agent Exposure](../integrations/agent-exposure.md)),
  so a live channel on a paused or archived agent never reads as Live. Public live channels sort
  first and are tinted. Publish and Unpublish sit on the row; nothing goes live except by that
  explicit step. Schedules are triggers and show on the agent row, not in the channel list.
- **No flag, one page.** It shipped behind `agents_home` and graduated with the old registry,
  single-form New agent page and Exposures page deleted. `/exposures` redirects to
  `/agents?view=channels`; the agent view and edit pages are unchanged.
- **People are counted only where a channel names them.** "People reached" counts distinct
  end-user principals behind a channel's sessions: Slack senders recorded as session participants,
  and the virtual user stamped on each `input.message` (public chat, signed-in AG-UI, PACT A2A).
  Webhook, API, FCP, anonymous AG-UI and plain A2A carry no person, so those channels show
  sessions only rather than an estimated head count. The org figure counts each person once.
- **First reply is the median of each session's first exchange.** The time from a session's first
  `input.message` to its first `output.message.completed`, median per channel over 7 days. The
  strip shows the fastest live channel.
- **Setup not finished comes from provenance, not a stored flag.** Adopting a guided example tags
  the agent `example:<name>`. An adopted guided example with no active trigger is a Needs
  attention item linking back to the setup page. Agents adopted before the tag existed are not
  flagged.
- **Describe first, and every path makes a normal agent.** The New agent page opens on the agent
  builder (`POST /v1/agents/draft`), which edits a draft and never creates anything; the
  browser creates the agent with the ordinary calls on confirm. Examples, a Blank form sharing the
  same draft, and package import are the other tabs. Ways in become draft channels and the
  schedule a schedule trigger, so nothing takes traffic until published. Slack needs a workspace
  choice, so a draft that asks for it lands on the Slack channel form after create. The builder may
  only pick capabilities the org offers that need no config and are not high risk; see
  `crates/server/src/domains/agents/draft.rs`.

## Not built yet

- Test in Playground from an unsaved draft: the playground needs a saved agent.
