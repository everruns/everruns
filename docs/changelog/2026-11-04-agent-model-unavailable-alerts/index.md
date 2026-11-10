---
title: "Health and Slack say when an agent's model is unavailable"
description: "An agent whose model is off, unlisted, not a chat model or on a disabled provider opens a health issue, and channels say the agent needs fixing."
date: 2026-11-04
release: "0.51.0"
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, agent]
links:
  docs: [/features/ui/, /integrations/slack/]
  prs: [everruns/everruns#5221, everruns/everruns#5225, everruns/everruns#5226, everruns/everruns#5249]
---

An agent set to an unusable model no longer fails quietly. A model is unusable when it
is turned off, missing from its provider's model list, not a chat model, or on a
turned-off provider, the same rule the model picker applies. A model the provider lists
under its own spelling, such as OpenRouter's `openai/gpt-6.1-sol`, counts as listed.

**In the UI:** Settings → Health and the bell open an issue that names the agent, model,
reason and fix, and tells the agent's authors. A missing model's issue says which list
and id it checked and offers **Sync models**. A broken organization default is one
issue, not one per agent. See [Overview](/features/ui/#overview-page).

**In channels:** when a turn fails for that reason, [Slack](/integrations/slack/) and the
other channels post "This agent is not set up correctly, so it cannot answer right now"
in place of the generic notice.
