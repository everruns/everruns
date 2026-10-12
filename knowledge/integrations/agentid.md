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
   end-user virtual user and a runtime session. Implemented for Public Chat;
   see [Consumer sign-in](#consumer-sign-in).
3. **Outbound authorize helper**: an Everruns agent finishes another app's
   AgentID waiting page with its own AgentMail inbox. Implemented as the
   experimental `agentid` capability; see
   [Outbound authorize helper](#outbound-authorize-helper).

## Identity boundary

An AgentID subject is an agent. It becomes an end-user
[virtual user](../runtime-resources/virtual-users.md), never a management
user: no `users` row, no personal access token, no `/login`, and no email
linking of `email` or `owner_email` onto an Everruns account.

Subjects proven by AgentID's own discovered keys bind under provider `agentid`
with realm `https://auth.agentid.com`. A channel whose keys are configured by
its owner keeps the hashed verifier realm of plain `oidc` (TM-AUTH-031), so it
can never write `agentid` bindings.

## Consumer sign-in

An agent with no token of its own signs in through its browser, the same way a
person uses "Sign in with Google". Source:
[`crates/server/src/api/agentid_login/mod.rs`](../../crates/server/src/api/agentid_login/mod.rs),
[`crates/server/src/storage/agentid/mod.rs`](../../crates/server/src/storage/agentid/mod.rs).

- **Deployment client.** One registered AgentID client per deployment, from
  `AGENTID_CLIENT_ID` and `AGENTID_CLIENT_SECRET`; the redirect URI is
  `{base_url}/v1/agentid/callback` unless `AGENTID_REDIRECT_URI` says
  otherwise. No client is registered per channel. The secret stays out of logs
  and, in CI, out of `GITHUB_ENV` and `GITHUB_OUTPUT`. Independent of
  `AUTH_MODE`.
- **Opt-in per channel.** Only a live Public Chat channel whose own auth is the
  AgentID preset offers "Continue with AgentID". Any other channel refuses the
  login route, so a runtime session never bypasses a channel's chosen auth.
- **Flow.** `GET /v1/channels/{id}/public-chat/agentid/login` stores
  server-side state (channel, PKCE verifier, nonce, `login_hint`; keyed by the
  state's hash, single use, ten minutes) and redirects to AgentID. The
  callback requires `iss` (RFC 9207), exchanges the code, verifies the
  id_token (ES256, `iss`, exact `aud`, then `nonce` and `actor_type`), reads
  `/v0/userinfo` for the same `sub`, and resolves the end-user virtual user by
  `sub`. The browser returns to the chat page with the existing short runtime
  token in the URL fragment; the page moves it into session storage. AgentID
  has no refresh token, and the runtime token is not lengthened to make up
  for it: a new sign-in renews it, inside AgentID's remembered approval.
- **Scopes.** `openid email profile`. `owner_sub` comes with `profile` and is
  required; a sign-in without it fails. `owner_profile owner_email` are
  requested only with `AGENTID_OWNER_SCOPES=true`, and then `owner_email` is
  stored as a contact only.
- **Names.** The virtual user's display name is the agent's `name`, else
  `preferred_username`, else the inbox's local part; never `owner_name`.
- **Per-owner cap.** An org caps how many active agents one `owner_sub` may
  sign in (`agentid_agents_per_owner` on the organization, default 5). An
  agent that already has an account always signs back in.
- **Directory link.** `GET /v1/agentid/initiate-login` is the URL to list in
  the AgentID directory. AgentID appends only `iss` and `login_hint`, which
  name no channel, so it signs in to `AGENTID_DEFAULT_CHANNEL` when that is
  set and otherwise creates nothing and says so: no channel, no account. It
  never sends the browser to `/login`.

## Outbound authorize helper

The reverse direction: an Everruns agent signing in to someone else's app as
itself. The app shows an AgentID waiting page with an `auth_token`; the agent
calls `agentid_authorize`, which posts
`{"auth_token": ..., "accept_disclosure": true}` to AgentMail's
`POST /v0/inboxes/{inbox_id}/authorize` and returns AgentMail's `api_key_id`
and `instructions`. Source:
[`crates/integrations-experimental/src/agentid/`](../../crates/integrations-experimental/src/agentid/).

- **Whose key.** The AgentMail API key and inbox id are an `agentmail`
  connection on the agent's service virtual user. The tool resolves it with
  `UserConnectionResolver::get_service_api_key_connection`, which reads only
  the responding agent's service identity: never the invoking end user's
  connections, never a management user's, and no fallback (TM-TOOL-058). End
  users cannot hold this connection for the agent.
- **What is sent.** Only the auth token and the disclosure acceptance, to one
  fixed AgentMail origin, through the egress boundary with DNS pinning and the
  session's network policy. No mail send or read.
- **Validation.** Saving the connection checks format only (key, inbox as an
  address) and makes no AgentMail call; a wrong key surfaces on first use.
- **Rollout.** No feature flag: an agent signs in only after someone adds the
  capability and an AgentMail connection, which is already the opt-in.
