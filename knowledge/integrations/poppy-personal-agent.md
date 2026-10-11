---
type: Specification
title: "Poppy Personal Agent"
description: "Everruns agents as Personal Agent Protocol (Poppy) personal agents: one deployment identity, pairwise User IDs, the poppy_client capability."
tags:
  - everruns
  - integrations
---
# Poppy Personal Agent

## Abstract

The [Personal Agent Protocol](https://personalagentprotocol.org) ("Poppy",
draft 0.1) has two ends: the company and the person's own agent. The
[Poppy Channel](poppy-channel.md) makes an Everruns agent a company's agent.
This is the other end: an Everruns agent, Platform Chat first, acting as a
person's personal agent at any company that publishes
`/.well-known/poppy.json`.

## Goals

1. An agent with the `poppy_client` capability can find a company by domain,
   start a conversation with its agent, read replies, hand off to a person and
   close, with nothing to register at the company.
2. A company cannot correlate a person across companies, and cannot tell
   which org or session a request came from beyond the User ID it was given.
3. Keys and Session Tokens never reach a worker or the model.

## Non-Goals

1. Sign-in (spec 5). Sessions are anonymous for now; a company that needs an
   account answers that it needs sign-in, and the agent says so.
2. Company APIs and operations (spec 6, 8), and event streams.

## Shape

- **Identity.** One personal agent per deployment. The server publishes its
  client metadata document at `/v1/poppy/agent.json` (that URL is the
  `client_id`, under the deployment's public API base) and its ES256 key at
  `/v1/poppy/jwks.json`. Companies see the client name "Everruns".
- **User IDs.** Random and pairwise per (org, session owner, company issuer),
  each with its own DPoP key. Keys are encrypted at rest like other signing
  keys.
- **Tools.** `poppy_discover`, `poppy_send` (starts the Session and the
  conversation on first use), `poppy_read`, `poppy_handoff`, `poppy_close`.
  The capability is in `crates/capabilities`; it only describes calls.
- **Where the protocol runs.** In the control plane, behind the
  `worker_poppy_call` internal command. The worker's tool sends the call and
  gets the company's answer back. Conversation state (Session, conversation
  id, event cursor, encrypted token) is stored per (session, company domain).
- **Network.** Every request goes through the egress boundary with DNS
  pinning; companies must be HTTPS domains, and authenticated calls do not
  follow redirects.

## Decisions

- The protocol is server-side, not in core, because the deployment key and
  the per-person keys must stay in one trusted place. A worker holding them
  would hand every company identity to every sandbox.
- Message ids come from the tool call id, so a retried tool call is a
  protocol retry (spec 7.3) and the company answers it once.
- A company's refusal reaches the model as text ("{domain} refused the
  {step}: {code} ({description})"); the agent can explain it to the person.
- An expired Session is renewed once on `401`; a `410` on events resets the
  cursor.

## Interoperability

Two domain tests drive the client end to end. One talks to our own Poppy
channel. The other talks to [pi-poppy](https://github.com/richardanaya/pi-poppy)'s
`CompanyServer` with DPoP nonces required, so an independent company judges
our client metadata, assertions and proofs; it runs from
`scripts/poppy-interop.sh` and in the same CI step as the channel checks.
A third, `real_model_asks_pi_poppy_company`, runs when `OPENAI_API_KEY` is set
too: gpt-6.1-sol gets the `poppy_client` tools and a person's question, and
its calls go through the worker's invoker and the server's command to the
same company, so it checks that a real model uses the tools as described and
relays the company's answer.

## Security

Threats and mitigations: `TM-POPPY-*` in
[Threat Model](../security/threat-model.md).
