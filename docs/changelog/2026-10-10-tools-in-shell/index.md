---
title: "Tools in Shell runs an agent's tools from Bashkit scripts"
description: "The tools_in_shell capability moves most tools into one tools shell command, so a script can chain calls, preview them with tools plan and ask approval first."
date: 2026-10-10
kind: feature
significance: major
products: [platform, cloud]
surfaces: [agent, ui]
links:
  docs: [/capabilities/tools-in-shell/]
  prs: [everruns/everruns#4385, everruns/everruns#4386, everruns/everruns#4388, everruns/everruns#4435, everruns/everruns#4439, everruns/everruns#4443, everruns/everruns#4454, everruns/everruns#4501]
---

The **Tools in Shell** capability (`tools_in_shell`) moves most of an agent's
tools, MCP servers included, from the model's tool list into one `tools`
command in the Bashkit shell. One script can call several tools and filter
results with `jq` before the model sees them:

```bash
tools github list-pulls '{"repo":"acme/api","state":"open"}' | jq -r '.[].number'
```

Every call goes through the turn's tool policy. A script that would make a call
needing approval is held before it starts. `tools plan` lists a script's calls
and their risk without running anything. Tools with no risk hints are rated
once, which **Rate unlabeled tools** can turn off. Each call shows in the
session timeline under its shell call.

See [Tools in Shell](/capabilities/tools-in-shell/) and [#4385](https://github.com/everruns/everruns/pull/4385), [#4388](https://github.com/everruns/everruns/pull/4388), [#4435](https://github.com/everruns/everruns/pull/4435), [#4439](https://github.com/everruns/everruns/pull/4439), [#4443](https://github.com/everruns/everruns/pull/4443), [#4501](https://github.com/everruns/everruns/pull/4501).
