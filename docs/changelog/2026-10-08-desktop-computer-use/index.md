---
title: "Desktop computer use on E2B and Daytona sandboxes"
description: "Experimental capabilities run the computer tool on a full Linux desktop in an E2B or Daytona sandbox, now with zoom, cursor, mouse button and hold-key actions."
date: 2026-10-08
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/computer-use/]
  prs: [everruns/everruns#4244, everruns/everruns#4365, everruns/everruns#4367]
---

![Everruns, E2B and Daytona side by side, announcing desktop computer use on both sandboxes](./integration@2x.png)

Agents can now operate a full Linux desktop instead of only a browser page. The
experimental `computer_use_desktop` capability runs the provider-neutral
`computer` tool on an E2B desktop sandbox, and `computer_use_daytona` runs it on
a Daytona sandbox through Daytona's Computer Use API. The session creates the
desktop on first use, keeps it between calls and deletes it when its lease ends.

The `computer` tool also gained zoom, cursor position, mouse button down and up,
hold-key and modified drag actions on Daytona, E2B and Browserless.

Connect E2B or Daytona first, then add one computer-use capability to the agent;
only one can provide the `computer` tool.

See [Computer use](/capabilities/computer-use/) and [#4244](https://github.com/everruns/everruns/pull/4244), [#4365](https://github.com/everruns/everruns/pull/4365), [#4367](https://github.com/everruns/everruns/pull/4367).
