---
type: Decision
title: "Bound Secrets"
description: "Secrets bound to one URL and auth form: write-only values the platform puts on outbound HTTP requests while the agent only ever sees a placeholder."
tags:
  - everruns
  - security
  - secrets
  - bashkit
---
# Bound Secrets

> Status: **Approved 2026-10-12.** Steps 1 and 3 built; step 2 (in-chat card) next.

## Problem

A Bashkit agent that calls a regular HTTP API needs a key. Today the only
route is a session secret that the model reads with `secret_store get`
([session storage](../../crates/core/src/host/session_services/capabilities/session_storage.rs))
and pastes into a curl command or a file. The key then lives in the
transcript, tool events, traces, the workspace and the Files view. Session
secrets are also per session, so every new session, and every trigger that
creates one, needs the key again.

## Decision

A **bound secret** is a secret stored with where it may be sent and how. The
model only handles a placeholder; the platform puts the real value on the
wire, and only for requests to the bound URL.

| Field | Meaning |
|---|---|
| `name` | Env var the shell sees, e.g. `JEV_API_KEY` |
| value | Write-only, envelope-encrypted like every secret ([encryption](encryption.md)) |
| `url` | `scheme://host[:port][/path-prefix]` the value may be sent to |
| `auth` | `bearer` (`Authorization: Bearer <v>`) or `header` with a header name |
| scope | `session`, or `agent` (every session and trigger of the agent) |

The agent writes ordinary code:

```bash
curl -sS https://api.typesafe.ai/v1/systemone \
  -H "Authorization: Bearer $JEV_API_KEY" --data @req.json
```

`$JEV_API_KEY` holds `bk_placeholder_<hex>`. Bashkit's
`credential_placeholder` replaces the placeholder in outbound headers only for
requests matching the bound URL, after its allowlist check and per redirect
hop. Elsewhere the placeholder goes out unchanged and is worthless.

### Rules

- **Write-only.** No API, tool or internal command returns a bound value.
  `secret_store get` refuses a bound name and points at `$NAME`;
  `secret_store list` shows name, URL and auth so the model knows what to use.
- **Agent scope.** Stored per agent and resolved from the session's agent, so
  shared sessions and per-invocation trigger sessions get it. A session secret
  with the same name wins.
- **No network by itself.** The bound URL must still pass the agent's network
  access list and the egress policy; binding never widens egress.
- **Rebinding clears the value.** Changing an existing secret's URL or auth
  requires the value again, so a key is never redirected silently.
- **Known-value redaction.** Bound values are redacted from shell output, the
  same way MCP credential injection redacts injected values
  ([executor](../../crates/core/src/mcp/executor.rs)), so an API that echoes
  headers back still cannot leak the key into the transcript.
- **Headers only.** Query-string or body credentials are not covered.

## Setting a bound secret

1. **In chat, like a connection.** The agent calls `request_secret` (name,
   URL, auth, scope, reason). The chat shows a **Secret** card: masked field,
   bound URL, scope, Save / Decline. The browser posts the value to the API;
   it never passes through the model or the transcript. The agent is told
   only "`JEV_API_KEY` saved, use `$JEV_API_KEY`".
2. **Over MCP.** The platform MCP tool `session_set_secret` already collects
   values through URL elicitation, so the value never passes through the
   calling model ([elicitation](../../crates/server/src/api/mcp_endpoint/elicitation.rs)).
   It gains optional `url`, `auth`, `header` and an agent scope; the signed
   elicitation token carries the binding.
3. **API, CLI and agent page.** `PUT /v1/sessions/{id}/storage/secrets`
   accepts bindings; `/v1/agents/{id}/secrets` manages agent-scoped ones; the
   agent page gets a **Secrets** section.

## Replay and redirection protection

- **Single-use requests.** `request_secret` creates a pending request (id,
  session, agent, name, binding, scope, random nonce, 15-minute expiry). The
  card submits `{request_id, nonce, value}` only. The server checks pending,
  unexpired, same session, and that the submitter may write the session (and
  manage the agent for agent scope), then stores and marks it consumed in one
  transaction. Replays, second tabs and stale cards get "already used or
  expired".
- **Binding is server-side.** URL, auth, name and scope come from the request
  row (or the signed elicitation token), never from the browser payload.
- **Injected requests.** The card shows the bound host in full, and the host
  must already pass the agent's network access list and egress policy.
- **Placeholders do not replay.** A fresh placeholder per shell call; one
  copied out of a transcript maps to nothing.
- **API sets** use normal auth and the `/v1/commands` idempotency key.

## Threats

TM-BASH-030 (bound value disclosed to the model), TM-BASH-031 (bound value
sent to another host) and TM-BASH-032 (replayed or forged secret request) in
the [threat model](threat-model.md).

## Rollout

1. Store, API, Bashkit injection and redaction: binding columns on
   `session_secrets`, `agent_secrets` table, write-only `secret_store`.
2. `request_secret` tool, pending requests, in-chat Secret card, agent page
   Secrets section.
3. MCP `session_set_secret` binding and agent scope.
4. Later: the same rule at the egress boundary, for managed sandboxes and
   `web_fetch`.
