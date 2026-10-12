---
title: Computer use capability lets agents operate a browser screen
description: The experimental computer_use capability drives a Browserless page with clicks, typing and screenshots, using native OpenAI or Anthropic tools when available.
date: 2026-10-03
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent]
release: "0.35.0"
links:
  docs: [/capabilities/computer-use/]
  prs: [everruns/everruns#3922, everruns/everruns#3968, everruns/everruns#4064]
---

The experimental `computer_use` capability lets an agent work a screen the way
a person does. Its `computer` tool takes an action (click, type, key, scroll,
drag, navigate or screenshot) and returns a screenshot, so any image-capable
model can use it. The screen is the session's Browserless browser.

On models with a native computer tool, the agent uses the provider's tool
instead; set `native_tools` to `false` to always use the function tool.
`max_actions_per_session` caps actions per session, and navigation passes the
egress guard. No call waits for a person by default: add `soft_approval` to
record confirmations, or `tool_approval` to approve every call.

See [Computer use](/capabilities/computer-use/). PRs [#3922](https://github.com/everruns/everruns/pull/3922),
[#3968](https://github.com/everruns/everruns/pull/3968), [#4064](https://github.com/everruns/everruns/pull/4064).
