---
title: Agent page rebuilt as one view-and-edit workspace
description: The agent page shows the system prompt as the page with settings in a side column, and you edit everything in place with one Save or Discard.
date: 2026-09-29
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.33.0"
visual: "Agent page in view mode showing the system prompt and the config column, then the same page in edit mode with Save changes and Discard; route /agents/{agent}; crop to the main workspace."
links:
  docs: [/features/ui/]
  prs: [everruns/everruns#3917]
---

The agent page is now one workspace. The system prompt fills the page and a
narrow column holds the rest of the configuration. Select **Edit** to change
the prompt and settings in place, then **Save changes** or **Discard** for the
whole agent. Changes apply to new sessions.

MCP servers, credentials, branding, starter files, network access, token usage
and health check open as side sheets from **More**. Version history and
archive move to the header's **More actions** menu. Old `/edit` links open the
page in edit mode, and `/sessions` opens the Sessions tab.

See [Agent page](/features/ui/#agent-page). PR [#3917](https://github.com/everruns/everruns/pull/3917).
