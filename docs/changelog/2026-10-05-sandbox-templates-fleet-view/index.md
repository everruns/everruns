---
title: "Sandbox Templates and a Sandboxes fleet view"
description: "Reusable Sandbox Templates with immutable revisions decide where an agent's commands run, and a fleet page lists every sandbox in the organization."
date: 2026-10-05
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui, api, cli]
release: "0.42.0"
links:
  docs: [/features/sandbox-templates/]
  prs: [everruns/everruns#4199, everruns/everruns#4246, everruns/everruns#4262]
visual: "Sandboxes fleet page with summary tiles, timeline chart and the Needs attention filter (/sandboxes), full width"
---

Sandbox Templates choose where commands run: the Bashkit Virtual Workspace or a
managed provider such as Daytona or E2B, with credentials, durability and
lifecycle policy. Editing creates an immutable revision; existing sessions keep
their original revision.

Create templates under **Sandbox Templates**. **Sandboxes** lists the fleet,
lifecycle timeline and **Needs attention** filter. The detail drawer offers
pause, resume and delete. The `/v1/sandboxes` API exposes the fleet, statistics
and timeline.

See [Sandbox Templates](/features/sandbox-templates/).

## Update 2026-11-21

Idle sandboxes show their last recorded state. Admins can check provider status.
The idle sweep retries failed pauses in shards.

## Update 2026-11-22

Provider status refreshes automatically, with a five-minute cooldown.

## Update 2026-11-23

Idle sessions with a saved workspace checkpoint show as **Recoverable** when
their provider instance is gone. They stay outside **Live now** and **Needs
attention**; active-session losses and unavailable recovery still need attention.

The attention filter keeps its selection.
