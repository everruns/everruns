---
title: "Save shell scripts on agents and run them from triggers"
description: "Agents keep named shell scripts that run in Bashkit with their tools, and schedule or webhook triggers can run one with the event payload and no model call."
date: 2026-10-09
kind: feature
significance: notable
products: [platform, cloud, cli]
surfaces: [api, cli, agent]
release: "0.46.0"
links:
  docs: [/capabilities/tools-in-shell/#saved-scripts, /features/agent-triggers/]
  prs: [everruns/everruns#4390, everruns/everruns#4400, everruns/everruns#4403, everruns/everruns#4423]
---

An agent can own saved scripts: a name, a description, an optional input
schema and a shell body. Manage them with `everruns agents scripts` or under
`/v1/agents/{agent_id}/scripts`; every change is in the agent's history.

Inside the shell, `tools scripts` lists and runs them with the caller's tools
and identity, and with `manage_scripts` on the agent can save new ones.

A schedule or webhook trigger can run a script without asking the model:

```json
{ "script": { "script": "triage-prs", "input": { "pr": "{{payload.pull_request}}" } } }
```

Input strings are templates over the event, so a webhook payload reaches the
script.

See [Saved scripts](/capabilities/tools-in-shell/#saved-scripts),
[Agent triggers](/features/agent-triggers/) and [#4390](https://github.com/everruns/everruns/pull/4390), [#4400](https://github.com/everruns/everruns/pull/4400), [#4403](https://github.com/everruns/everruns/pull/4403), [#4423](https://github.com/everruns/everruns/pull/4423).
