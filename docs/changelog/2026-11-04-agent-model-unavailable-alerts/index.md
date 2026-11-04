---
title: "Health and Slack say when an agent's model is unavailable"
description: "An agent whose model is off, unlisted, not a chat model or on a disabled provider opens a health issue, and channels say the agent needs fixing."
date: 2026-11-04
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, agent]
links:
  docs: [/features/ui/, /integrations/slack/]
  prs: [everruns/everruns#5221, everruns/everruns#5225, everruns/everruns#5226]
---

An agent set to an unusable model no longer fails quietly. A model counts as
unusable when it is turned off, no longer listed by its provider, not a chat model, or
on a turned-off provider: the same rule the model picker applies when it says "The
configured model is unavailable".

**In the UI:** Settings → Health and the bell open an issue that names the agent, model,
reason and fix, and tells the agent's authors. A broken organization default model is
one issue, not one per agent. Fixing the cause resolves it. See [Home](/features/ui/#home).

**In channels:** when a turn fails for that reason, [Slack](/integrations/slack/) and the
other channels post "This agent is not set up correctly, so it cannot answer right now"
in place of the generic notice. No model or provider names are shown.

Models an old sync wrongly marked "no longer listed" are listed again.
