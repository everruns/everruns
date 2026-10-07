---
type: Specification
title: "AgentID"
description: "AgentID (AgentMail's OIDC issuer for AI agents): channel preset, consumer sign-in, and agents finishing other apps' AgentID sign-ins."
tags:
  - everruns
  - integrations
  - identity
---

# AgentID

[AgentID](https://www.agentid.com) is an OpenID Connect issuer for AI agents,
run by AgentMail. An agent's identity is an AgentMail inbox: `sub` is an opaque
id for that inbox, stable until the inbox is deleted, and `actor_type` is always
`"agent"`. `owner_sub` (with the `profile` scope) is shared by every agent one
human runs. Tokens are ES256, last ten minutes, and have no refresh.

Everruns supports AgentID in three independent slices:

1. **Channel preset**: a channel accepts AgentID bearer tokens. Implemented;
   see [Channel Authentication](channel-auth.md#agentid-preset).
2. **Consumer sign-in**: a completed AgentID browser sign-in becomes an
   end-user virtual user and a runtime session. Planned.
3. **Outbound authorize helper**: an Everruns agent finishes another app's
   AgentID waiting page with its own AgentMail inbox. Planned.

## Identity boundary

An AgentID subject is an agent. It becomes an end-user
[virtual user](../runtime-resources/virtual-users.md), never a management
user: no `users` row, no personal access token, no `/login`, and no email
linking of `email` or `owner_email` onto an Everruns account.

Subjects proven by AgentID's own discovered keys bind under provider `agentid`
with realm `https://auth.agentid.com`. A channel whose keys are configured by
its owner keeps the hashed verifier realm of plain `oidc` (TM-AUTH-031), so it
can never write `agentid` bindings.
