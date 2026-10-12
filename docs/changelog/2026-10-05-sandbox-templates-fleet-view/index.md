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

A Sandbox Template names where an agent's commands run: the Bashkit Virtual
Workspace or a managed provider such as Daytona or E2B, with its credential
source, durability and lifecycle. Editing a template creates a new immutable
revision, and existing sessions keep the revision they started with. An agent's
sandbox policy decides whether a session can pick or override the template.

**In the UI:** open **Sandbox Templates** and select **New Sandbox Template**.
The **Sandboxes** page lists every sandbox across providers and states, with a
lifecycle timeline, a **Needs attention** filter and pause, resume and delete in
the detail drawer. Both sit in the collapsible **Sandboxes** sidebar group.

**In code:** `GET /v1/sandboxes` lists the fleet; `/stats` and `/timeline` back
the page.

See [Sandbox Templates](/features/sandbox-templates/) and [#4199](https://github.com/everruns/everruns/pull/4199), [#4246](https://github.com/everruns/everruns/pull/4246), [#4262](https://github.com/everruns/everruns/pull/4262).
