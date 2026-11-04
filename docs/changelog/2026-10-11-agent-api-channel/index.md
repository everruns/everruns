---
title: "Call one agent from code with an Agent API key"
description: "An Agent API channel gives an application a key that reaches one agent's sessions, with streaming, approvals, end users, browser origins and spend limits."
date: 2026-10-11
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui, api, code]
release: "0.48.0"
links:
  docs: [/how-to/call-your-agent-from-code/, /features/agent-api/]
  prs: [everruns/everruns#4438, everruns/everruns#4490, everruns/everruns#4508, everruns/everruns#4511, everruns/everruns#4514, everruns/everruns#4516, everruns/everruns#4527, everruns/everruns#4559]
---

**In the UI:** add an **Agent API** channel to an agent, publish it, and create a
key in its **Agent keys** card. The secret (`evr_ak_…`) is shown once. An
application holding it can call this one agent and nothing else; keys rotate
with an overlap window and can be revoked.

![Agent keys card on an Agent API channel with two active keys and Rotate and Revoke](./agent-api-keys@2x.png)

**In code:** routes under `/v1/channels/{channel_id}` start sessions, send
messages, follow events over SSE, answer questions, approve tools and cancel
turns. A key sees only the sessions it started. Channel settings add end users
through your own identity provider, browser origins, `Idempotency-Key` retries,
a daily spend limit per caller, and access for organization members.

See [Call your agent from code](/how-to/call-your-agent-from-code/),
[Agent API](/features/agent-api/) and [#4438](https://github.com/everruns/everruns/pull/4438), [#4490](https://github.com/everruns/everruns/pull/4490), [#4508](https://github.com/everruns/everruns/pull/4508), [#4511](https://github.com/everruns/everruns/pull/4511), [#4514](https://github.com/everruns/everruns/pull/4514), [#4516](https://github.com/everruns/everruns/pull/4516).
