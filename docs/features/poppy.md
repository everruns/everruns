---
title: Personal Agent Protocol (Poppy)
description: Let people's own agents talk to your Everruns agent over the Personal Agent Protocol, and let an Everruns agent act as a person's agent at other companies.
sidebar:
  label: Poppy
appliesTo: [framework, platform, cloud]
---

The [Personal Agent Protocol](https://personalagentprotocol.org) ("Poppy") is
an open standard for a person's own agent to do business with a company: find
the company by its domain, start a Session, talk to the company's agent, and
ask for a person. Everruns implements
[draft 0.1](https://personalagentprotocol.org/docs/spec) on both ends:

- **Company side**: a `poppy` [channel](/features/channels/) makes an Everruns
  agent a company's agent. Any personal agent that follows the spec can reach
  it, with nothing to register.
- **Personal side**: the [`poppy_client`](/capabilities/poppy-personal-agent/)
  capability lets an Everruns agent act for a person at any company that
  publishes `/.well-known/poppy.json`. Platform Chat has it, and Framework apps
  get it with `PoppyPersonalAgent`.

Poppy is not A2A or PACT. It is standard OAuth pieces (client metadata
documents as agent identity, a JWT bearer grant, DPoP on every request) plus
its own conversation API. For agent-to-agent calls between systems you own, use
[A2A](/features/a2a/); for PACT personal agents, see
[A2A personal agents](/features/a2a/#personal-agents-pact).

## Be a company's agent

Create a `poppy` channel on the agent that should answer, then publish it:

```bash
EVERRUNS=https://your-everruns-host/api
curl -X POST "$EVERRUNS/v1/agents/$AGENT_ID/channels" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "channel_type": "poppy",
    "channel_config": {
      "domains": ["shop.example.com"],
      "organization_name": "Example Shop"
    }
  }'
curl -X POST "$EVERRUNS/v1/agents/$AGENT_ID/channels/$CHANNEL_ID/publish" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

Then redirect your own discovery document to the channel. Everruns serves
everything else:

```text
https://shop.example.com/.well-known/poppy.json
  -> 302 https://your-everruns-host/api/v1/channels/{channel_id}/poppy/poppy.json
```

| Field | Description |
| --- | --- |
| `domains` | Required. Your domains as lowercase host names, each redirecting its `/.well-known/poppy.json` here. The first is the one `poppy.json` names. |
| `organization_name` | The company name personal agents see. Defaults to the agent's name. |
| `allowed_agents` | Personal agents allowed to start Sessions, by `client_id`. Absent means any personal agent that proves its identity. |
| `blocked_agents` | Personal agents refused even when `allowed_agents` is absent. |
| `rate_limit_per_minute` | Optional per-IP limit for this channel. |

The channel holds no secrets: Everruns generates the key that signs Session
Tokens and keeps it encrypted.

### What the channel serves

Everything is under the channel's base URL,
`/v1/channels/{channel_id}/poppy`, which is also the OAuth issuer:

| Route | What it is |
| --- | --- |
| `GET poppy.json` | The discovery document your redirect points at. |
| `GET .well-known/oauth-authorization-server` | Sign-in server metadata, listing your domains. |
| `POST oauth/token` | Starts or renews a Session (JWT bearer grant, `private_key_jwt`, DPoP). |
| `POST oauth/revoke` | Token revocation (RFC 7009). |
| `POST conversations`, `…/{id}/messages`, `GET …/{id}/events`, `POST …/{id}/handoff`, `…/{id}/close` | The conversation API. |

Each conversation is one Everruns session, owned by the personal agent and the
person's User ID, so you can read it in the session list like any other.
Personal agents see only the agent's replies, never tool calls, reasoning or
errors. A retried message returns the stored reply instead of starting a
second turn.

## Act for a person at a company

Add the `poppy_client` capability to an agent:

```json
{ "ref": "poppy_client", "config": {} }
```

The agent gets five tools (`poppy_discover`, `poppy_send`, `poppy_read`,
`poppy_handoff`, `poppy_close`) and can talk to any company by domain. On
Platform and Cloud, the deployment is one personal agent with a different User
ID per person at each company, and its keys and Session Tokens stay on the
server. In the Framework, the app is its own personal agent. See
[Poppy Personal Agent](/capabilities/poppy-personal-agent/) for both, and
[the Framework example](https://github.com/everruns/everruns/tree/main/examples/poppy-personal-agent)
for a runnable app.

## Interoperability

Draft 0.1 has no official conformance suite. Both ends are tested against
independent implementations:

- [pi-poppy](https://github.com/richardanaya/pi-poppy)'s personal agent, which
  maps every MUST, SHOULD and MAY of the draft to tests, drives a channel
  through discovery, a Session, a conversation, handoff, close and revocation.
- Datalayer's `pap company verify` checks the channel's discovery.
- Everruns agents with `poppy_client`, on the server and in the Framework, talk
  to pi-poppy's company, with its DPoP nonces on.

CI runs the first two and the scripted personal-agent checks on every change.
[`scripts/poppy-interop.sh`](https://github.com/everruns/everruns/blob/main/scripts/poppy-interop.sh)
also runs the real-model ones when `OPENAI_API_KEY` is set.

## Limits

- Sessions are signed out. Sign-in (`poppy:read` and `poppy:write` scopes,
  Account Tokens) and Direct Conversations are not implemented yet, so a
  company that needs an account says so.
- Asking for a person is answered by the agent: no one at the company joins
  from Everruns yet.
- Company APIs and the operations extension are not used on either side.
- Channels are created over the API; there is no setup screen yet.

## Implementation

- Company side:
  [`crates/server/src/channels/poppy`](https://github.com/everruns/everruns/tree/main/crates/server/src/channels/poppy)
  and its configuration in
  [`record/poppy.rs`](https://github.com/everruns/everruns/blob/main/crates/server/src/domains/agent_channels/record/poppy.rs).
- Personal side: the protocol client in
  [`crates/capabilities/src/capabilities/poppy_client`](https://github.com/everruns/everruns/tree/main/crates/capabilities/src/capabilities/poppy_client),
  used by the server's
  [`poppy_agent`](https://github.com/everruns/everruns/tree/main/crates/server/src/domains/poppy_agent)
  and by Framework apps.
