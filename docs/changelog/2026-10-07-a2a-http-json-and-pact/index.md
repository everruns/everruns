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

```bash
curl -sS -X POST "$EVERRUNS/v1/channels/$CHANNEL_ID/a2a/message:send" \
  -H "Authorization: Bearer $KEY" \
  -H 'Content-Type: application/a2a+json' \
  -d '{"message": {"role": "ROLE_USER", "messageId": "msg-1",
       "parts": [{"text": "Summarize the A2A spec in three bullets."}]}}'
```

Add `pact` to an A2A channel's config to accept personal agents under the PACT
1.0 Identity profile, with each conversation bound to the calling user. With
delegation on, the channel runs an OAuth device-code sign-in so a personal agent
can act as a company user through the configured MCP server, and delegated
replies carry a signed receipt.

See [A2A](/features/a2a/), [Personal agents (PACT)](/features/a2a/#personal-agents-pact)
and [#4322](https://github.com/everruns/everruns/pull/4322), [#4328](https://github.com/everruns/everruns/pull/4328), [#4350](https://github.com/everruns/everruns/pull/4350), [#4352](https://github.com/everruns/everruns/pull/4352).

Watch PACT account consent, a refused cancellation, and a signed receipt in a
local walkthrough with synthetic store data:

<iframe src="https://www.youtube-nocookie.com/embed/9FZiYv5PsWY?rel=0" title="PACT account consent and signed receipts in Everruns" loading="lazy" style="width:100%;aspect-ratio:16/9;border:0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe>

[Watch on YouTube](https://www.youtube.com/watch?v=9FZiYv5PsWY).
