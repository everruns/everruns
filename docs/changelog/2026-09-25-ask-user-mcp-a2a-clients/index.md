---
title: Ask User questions reach MCP, A2A and Tasks clients
description: A session waiting on ask_user now shows its question to MCP clients as a form elicitation, to A2A callers as a typed data part, and to Tasks clients.
date: 2026-09-25
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [agent]
release: "0.32.0"
links:
  docs: [/capabilities/ask-user/, /features/mcp/, /features/a2a/]
  prs: [everruns/everruns#3781, everruns/everruns#3780, everruns/everruns#3828]
---

When an agent waits on an `ask_user` question, clients outside the Everruns UI
can now read and answer it. Before, they saw a session that stayed working.

- Over `/mcp`, the question set is a form-mode elicitation. A client that never
  declared elicitation gets the declared defaults.
- Over inbound A2A, the task reports that input is required, with the question
  as text and as a data part under `everruns/ask_user`. Reply with a data part
  under `everruns/ask_user_answer`.
- Over MCP Tasks, `tasks/get` exposes the pending form through
  `inputRequests`, and `tasks/update` answers it.

See [Ask User](/capabilities/ask-user/), [MCP](/features/mcp/) and
[A2A](/features/a2a/). PRs [#3781](https://github.com/everruns/everruns/pull/3781), [#3780](https://github.com/everruns/everruns/pull/3780),
[#3828](https://github.com/everruns/everruns/pull/3828).
