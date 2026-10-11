---
title: "Agents home and a New agent page with an agent builder"
description: "The Agents page now shows run activity, channels and setup problems for each agent, and New agent drafts an agent from a description, an example or an import."
date: 2026-10-09
kind: feature
significance: major
products: [platform, cloud, cli]
surfaces: [ui, cli]
release: "0.47.0"
links:
  docs: [/features/ui/, /features/channels/]
  prs: [everruns/everruns#4399, everruns/everruns#4448, everruns/everruns#4457]
visual: "Agents home with per-agent run bars and the Needs attention list (/agents), plus the New agent page Describe tab (/agents/new)"
---

**In the UI:** the **Agents** page shows each agent's channels and triggers,
24-hour run bars and a **Needs attention** list for setup problems. Its
**Channels** view lists every channel in the organization with publish and
unpublish, people reached and fastest first reply. It replaces the Exposures
page; `/exposures` now redirects there.

**New agent** offers **Describe**, **Example**, **Blank** and **Import**.
Describe drafts the agent with the utility model, and **Create** makes the
agent with its draft channels and schedule trigger.

**In code:** `everruns agents draft` runs the same agent builder from the CLI.

See [Web UI](/features/ui/), [Channels](/features/channels/) and [#4399](https://github.com/everruns/everruns/pull/4399), [#4448](https://github.com/everruns/everruns/pull/4448), [#4457](https://github.com/everruns/everruns/pull/4457).
