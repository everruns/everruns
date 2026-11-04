---
title: "API keys a Bashkit agent uses but never sees"
description: "Bind an API key to one URL and the Bashkit shell can call that API with $NAME, while the model only ever sees a placeholder and never the key."
date: 2026-10-12
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.48.0"
links:
  docs: [/capabilities/bashkit-shell/#api-keys-the-agent-never-sees, /capabilities/session-storage/]
  prs: [everruns/everruns#4609]
---

**In the UI:** on an agent's **Credentials** tab, **Add binding** now offers
**HTTP API**: a shell variable, the HTTPS URL it may be sent to, and whether it
goes as a bearer token or a named header. With outbound HTTP on, the agent's
shell calls the API with `$NAME`. The platform swaps the real key into that
header only for requests to the bound URL and redacts it if a response echoes
it back.

![Agent Credentials sheet with an HTTP API binding for $JEV_API_KEY next to an MCP tool credential](./agent-credentials-http-api@2x.png)

**In the API:** [session secrets](/capabilities/session-storage/) accept `bindings` on
`PUT /v1/sessions/{session_id}/storage/secrets`, and the MCP tool
`session_set_secret` takes `url`, `auth` and `scope`, collecting the value on a
secure page.

See [Bashkit Shell](/capabilities/bashkit-shell/#api-keys-the-agent-never-sees).

## Update 2026-10-19

[Scoped credentials](/changelog/2026-10-19-scoped-credentials-secure-capture/) add virtual-user and organization ownership, explicit agent grants, form-field bindings and Web Fetch response capture. Hosted Bashkit resolves authorization again for each credential-bearing send. Secure value updates now require the observed resource revision; ordinary unbound session secrets retain their existing lifecycle.
