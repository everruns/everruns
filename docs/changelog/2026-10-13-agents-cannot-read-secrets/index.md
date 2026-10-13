---
title: "Agents can no longer read secrets back"
description: "Session secrets are now write-only to agents: secret_store stores, lists and deletes them, but no tool returns a value to the model."
date: 2026-10-13
kind: breaking
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/capabilities/session-storage/, /capabilities/bashkit-shell/#api-keys-the-agent-never-sees]
  prs: [everruns/everruns#PR]
---

Secrets an agent stores with `secret_store` are now write-only. The agent can
set, list and delete them, but `secret_store get` refuses every name, so a key
no longer reaches the transcript, tool events, traces or the workspace.

Capabilities that need a key, such as GitHub, Brave Search and the sandboxes,
keep working: they read their own secret on the server. An agent that read a
key to put it in a curl command should use a bound secret instead: add it as an
**HTTP API** credential on the agent's **Credentials** tab and send `$NAME` in
the auth header. See
[Bashkit Shell](/capabilities/bashkit-shell/#api-keys-the-agent-never-sees).
