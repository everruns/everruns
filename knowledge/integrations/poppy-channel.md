---
type: Specification
title: "Poppy Channel"
description: "Personal Agent Protocol (Poppy) inbound channel: a company's front door for personal agents."
tags:
  - everruns
  - integrations
---
# Poppy Channel

## Abstract

The [Personal Agent Protocol](https://personalagentprotocol.org) ("Poppy",
draft 0.1) lets a person's own agent work with a company for them: find the
company, start a Session, sign in to the person's account, call its APIs and
talk to its agent. A `poppy` channel makes an Everruns agent that company's
agent. Everruns hosts the whole front door at
`/v1/channels/{channel_id}/poppy`, and the company only redirects its own
`/.well-known/poppy.json` there.

Poppy is not A2A and not PACT. It is standard OAuth pieces (client metadata
documents, JWT bearer grants, DPoP) plus its own conversation API, so it is a
channel type of its own; it reuses PACT's per-channel signing keys and the
pinned client channel auth uses for outbound fetches.

## Goals

1. Any personal agent that follows the spec can reach a published agent, with
   no registration, and only the company's redirect to set up.
2. One channel is one company: its own issuer, keys, User IDs and tokens, so
   two companies hosted here can never share a Session or a token.
3. Personal agents prove who they are with their own keys, and every request
   proves possession of the key its token is bound to.
4. Conversations reuse sessions and their event log, with nothing internal
   (tool calls, reasoning, errors, channel notes) reaching the caller.

## Non-Goals

1. Being a personal agent. Platform Chat acting at other companies is a
   separate feature.
2. Company APIs. A hosted company exposes its agent; its own OpenAPI or MCP
   APIs, and the website joining a Session, are the company's.
3. Mediated sign-in (the personal agent sending the person's password). We
   hold no company credentials.

## Shape

Everything is under the channel's base URL, which is also the OAuth issuer
(with the server's API prefix, for example
`https://app.everruns.com/api/v1/channels/{id}/poppy`):

| Route | What it is |
|---|---|
| `GET poppy.json?domain=` | The discovery document for one of the channel's domains. The company's `https://{domain}/.well-known/poppy.json` redirects here. Without `domain`, the first one. |
| `GET .well-known/oauth-authorization-server` | RFC 8414 metadata with `poppy_domains`. Also served at the RFC 8414 location for an issuer with a path, `/.well-known/oauth-authorization-server{issuer path}` at the server root. |
| `POST oauth/token` | Starts or renews a Session (JWT bearer grant, `private_key_jwt`, DPoP). |
| `POST oauth/revoke` | RFC 7009. Nothing to revoke until sign-in exists. |
| `POST conversations`, `…/{id}/messages`, `GET …/{id}/events`, `POST …/{id}/handoff`, `…/{id}/close` | The conversation API (spec 7). |

Channel config (`record/poppy.rs`): `domains` (required, the first one is the
default), `organization_name`, `allowed_agents` / `blocked_agents` by
`client_id`, `rate_limit_per_minute`. It holds no secrets.

## Decisions

- **Identity.** A `client_id` is the HTTPS URL of the agent's client metadata
  document, which must name itself and keep its `jwks_uri` on its own host.
  Documents are fetched through the pinned, private-address-refusing client
  and cached five minutes.
- **Session Tokens** are ES256 JWTs signed with the channel's key, bound to
  the DPoP key of the proof that got them, one hour. They carry only what the
  personal agent already knows, so they are signed, not encrypted. Every
  request also reads the Session row, so ending a Session stops its tokens.
  Sessions end after seven days without a new token.
- **DPoP only.** Bearer Session Tokens are for MCP APIs alone, and the
  channel lists none.
- **Conversations are sessions.** One conversation is one Everruns session,
  owned by (channel, `client_id`, User ID). Event ids are `evt_{sequence}` of
  the session's own log; closing adds a final `evt_closed`. Message ids are
  claimed per owner with a content hash, so retries never start a second turn.
- **What the model reads** is the message text, then `data` as JSON, then the
  person's context as it stands; a message with only `context` starts no
  turn.
- **Long polls** return once the company side has something to say, not on
  the echo of the caller's own message, so `wait` on a send brings back the
  reply.
- **Handoff** to a person is answered by the agent, through a hidden note,
  that no one is available (the spec allows this). The conversation stays
  with the agent.

## Roadmap

1. Sign-in (device and direct), `poppy:read` / `poppy:write` mapped to the
   agent's tools and checked before a tool runs, `authorization` events,
   Account Tokens with revocation, and Direct Conversations.
2. Channel setup in the UI, a check that the company's redirect is in place,
   and public docs.
3. Operations extension, a person at the company joining from the Everruns
   UI, and `everruns-serve` support.

## Security

Threats and mitigations: `TM-POPPY-*` in
[Threat Model](../security/threat-model.md).
