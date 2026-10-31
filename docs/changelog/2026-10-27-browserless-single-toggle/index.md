---
title: Browserless settings collapse into one toggle
description: The three Browserless adoption toggles become one Browserless flag; agents pick computer use or browser use via capabilities.
date: 2026-10-27
kind: improvement
significance: minor
products: [platform, cloud]
release: "0.50.0"
surfaces: [ui, api]
links:
  docs: [/capabilities/browser-use/, /capabilities/computer-use/]
  prs: [everruns/everruns#4958]
---
The Browserless section of adoption settings showed three separate toggles:
Browserless, Browserless browser use, and Browserless computer use. They are
now a single Browserless toggle. Turning it on connects your Browserless
account and unlocks the `browser_use` and `computer_use` capabilities, which
agents pick individually with their own options.

```text
# Before: three adoption toggles
browserless, browserless_browser_use, browserless_computer_use

# After: one toggle; pick the surface per agent
browserless  # unlocks the browser_use and computer_use capabilities
```

See [Browser Use](/capabilities/browser-use/) and [Computer Use](/capabilities/computer-use/).
