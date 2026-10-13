---
title: "Sandboxes page with tabs, and tab links you can share"
description: "Sandboxes, Templates and provider accounts are now tabs of one Sandboxes page, and the Models, Web search and Sandboxes tabs each have their own URL."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
release: "0.48.0"
links:
  docs: [/features/sandbox-templates/]
  prs: [everruns/everruns#4653]
---

![Sandboxes page with Sandboxes, Templates and Provider accounts tabs](./sandboxes-tabs@2x.png)

**Sandboxes** is now one page organized like **Models**: a **Sandboxes** tab with the fleet
table and timeline, a **Templates** tab for [Sandbox Templates](/features/sandbox-templates/), and a **Provider accounts** tab for admins. The
sidebar has a single Sandboxes entry under Building instead of a collapsed group of three.

Tabs on the Models, Web search and Sandboxes pages are now real paths, such as
`/models/providers`, `/web-search/defaults` and `/sandboxes/templates`, so a tab can be
bookmarked, shared or opened in a new browser tab. Old links keep working: `?tab=` links,
`/sandbox-templates` and `/sandbox-provider-accounts` redirect to the new paths.

[#4653](https://github.com/everruns/everruns/pull/4653)
