---
title: "Agents can no longer read secrets back"
description: "Session secrets are now write-only to agents: secret_store stores, lists and deletes them, but no tool returns a value to the model."
date: 2026-10-13
kind: breaking
significance: notable
products: [platform, cloud]
release: "0.49.0"
surfaces: [api]
links:
  docs: [/capabilities/session-storage/, /capabilities/bashkit-shell/#api-keys-the-agent-never-sees]
  prs: [everruns/everruns#4676]
---

Secrets an agent stores with [`secret_store`](/capabilities/session-storage/)
are now write-only. The agent can set, list and delete them, but `get` refuses
every name, so a key no longer reaches the transcript, tool events, traces or
the workspace.

Before, a `secret_store` call like this returned the value:

```json
{
  "operation": "get",
  "name": "OPENAI_API_KEY"
}
```

```json
{
  "operation": "get",
  "name": "OPENAI_API_KEY",
  "value": "sk-...",
  "found": true
}
```

Now the same call fails with a tool error for every name, stored or not:

```text
Secret values are write-only: no tool can read one back. ...
```

Capabilities that need a key, such as GitHub, Brave Search and the sandboxes,
keep working: they read their own secret on the server. An agent that read a
key to put it in a curl command should use a bound secret instead: add it as an
**HTTP API** credential on the agent's **Credentials** tab and send it from the
shell, as described in
[Bashkit Shell](/capabilities/bashkit-shell/#api-keys-the-agent-never-sees):

```bash
curl -sS https://api.example.com/v1/items -H "Authorization: Bearer $API_KEY"
```
