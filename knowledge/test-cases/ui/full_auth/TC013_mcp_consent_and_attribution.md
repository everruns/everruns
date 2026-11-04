---
type: Test Case
title: "TC013: MCP consent and client attribution"
description: "Verify truthful MCP consent and distinct creator, message, history, and audit provenance when multiple clients share a session."
tags:
  - everruns
  - test-case
  - ui
  - full-auth
---
# TC013: MCP consent and client attribution

## Preconditions

- Local DB-backed stack with `AUTH_MODE=full` and a disposable user.
- Two registered OAuth clients, A and B, approved by that user.
- One client uses a loopback callback; another has multiple registered HTTPS callbacks.

## Steps

1. Open consent using the second HTTPS callback. Verify its selected host is shown,
   the name is self-declared, and `mcp` clearly grants read/write/run access across
   the user's organizations. Verify the localhost callback explains its trust limit.
2. Request `mcp:read`. Verify both GET and a POST with a valid consent token reject
   it without creating a grant. Blank scope remains `mcp` in consent and token responses.
3. Client A creates a session and sends a message; B sends another message and edits
   the title. Session details keep A as creator; each message and history entry
   shows its own client ID, grant ID, and self-declared name beside the human actor.
4. Send a UI message or legacy MCP-token message. It has no inferred client label.
   Attempt to supply `everruns_mcp_client` metadata or forged `clientInfo`; neither
   changes the server-derived provenance. Assistant output is not labeled as input.
5. Verify a foreign organization override fails. Revoke B and verify requests fail
   after the documented cache window. Existing authorization checks still apply.
6. Verify relevant audit entries retain their details and client provenance after
   asynchronous persistence. No access/refresh token, secret, or callback query is copied.
7. Use a client name containing HTML. Confirm consent and tooltips render text,
   with no image or script execution. Check the session tooltip on a narrow screen.

## Expected result

Creators remain immutable, individual actions remain distinguishable, and legacy
records stay unattributed. No client name or callback host is presented as verified
publisher identity. Attribution never changes authorization.
