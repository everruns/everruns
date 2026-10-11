---
title: "A2A channels serve HTTP+JSON and the PACT profiles"
description: "Every A2A channel also answers the A2A 1.0 HTTP+JSON binding, and a channel can accept personal agents through the PACT Identity and Delegated profiles."
date: 2026-10-07
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent, api]
release: "0.44.0"
links:
  docs: [/features/a2a/, /features/a2a/#personal-agents-pact]
  prs: [everruns/everruns#4322, everruns/everruns#4328, everruns/everruns#4350, everruns/everruns#4352]
---

Every A2A channel URL now also serves the A2A 1.0 HTTP+JSON binding:
`message:send`, `message:stream`, task get, list, cancel and subscribe, push
notification configs and the extended card. The Agent Card lists the new
interface, and auth, rate limits and signing are shared with JSON-RPC.

Add `pact` to an A2A channel's config to accept personal agents under the PACT
1.0 Identity profile, with each conversation bound to the calling user. With
delegation on, the channel runs an OAuth device-code sign-in so a personal agent
can act as a company user through the configured MCP server, and delegated
replies carry a signed receipt.

See [A2A](/features/a2a/), [Personal agents (PACT)](/features/a2a/#personal-agents-pact)
and [#4322](https://github.com/everruns/everruns/pull/4322), [#4328](https://github.com/everruns/everruns/pull/4328), [#4350](https://github.com/everruns/everruns/pull/4350), [#4352](https://github.com/everruns/everruns/pull/4352).
