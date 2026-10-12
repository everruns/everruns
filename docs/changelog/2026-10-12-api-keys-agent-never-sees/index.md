---
title: "API keys a Bashkit agent uses but never sees"
description: "Bind an API key to one URL and the Bashkit shell can call that API with $NAME, while the model only ever sees a placeholder and never the key."
date: 2026-10-12
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/capabilities/bashkit-shell/#api-keys-the-agent-never-sees, /capabilities/session-storage/]
  prs: [everruns/everruns#PR]
visual: "Agent Credentials tab with an HTTP API credential row ($JEV_API_KEY → https://api.typesafe.ai) next to an MCP tool credential (/agents/<id>?tab=credentials)"
---

**In the UI:** on an agent's **Credentials** tab, **Add binding** now offers
**HTTP API**: a shell variable, the HTTPS URL it may be sent to, and whether it
goes as a bearer token or a named header. With outbound HTTP on, the agent's
shell calls the API with `$NAME`. The platform swaps the real key into that
header only for requests to the bound URL and redacts it if a response echoes
it back.

**In the API:** session secrets accept `bindings` on
`PUT /v1/sessions/{session_id}/storage/secrets`, and the MCP tool
`session_set_secret` takes `url`, `auth` and `scope`, collecting the value on a
secure page.

See [Bashkit Shell](/capabilities/bashkit-shell/#api-keys-the-agent-never-sees).
