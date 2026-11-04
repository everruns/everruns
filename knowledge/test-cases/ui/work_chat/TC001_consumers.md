---
type: Test Case
title: "TC001: Work Chat - Isolated consumers and interface modes"
description: Verify hosted interface modes, independent consumer identity and workspace authorization.
---
# TC001: Work Chat - Isolated consumers and interface modes

## Description

Verify hosted interface modes, independent consumer identity and workspace authorization.

## Preconditions

A real deployment with authentication enabled (not `none`), registered OIDC
broker callback, TLS, isolated browser-credential database role, two independent
browser profiles and two verified consumers without organization memberships.
Publish one multi-chat channel and one single-with-threads channel with effective
coordinator tools. Enable workspace tools on the latter. Record fresh fixture
Agent/channel IDs and a start time; do not reuse pre-existing sessions.

## Test Data

| Input | Value |
| --- | --- |
| Consumers | Alice and Bob, no console membership |
| Main fixture | Fresh single-with-threads channel |
| Multi fixture | Fresh multi-chat channel |
| Private paths | `/workspace/memory`, `/workspace/../memory`, `/workspace/.tasks` |

## Steps

1. Open the single channel anonymously. Sign in through the configured broker.
   Verify no management cookie is sent to the hosted domain. Open another tab;
   verify both resolve the same newly-created main session ID.
2. Ask the real Agent to start a work thread. Within 90 seconds verify its task
   links a persisted child session with the same channel and consumer owner.
   Switch to the thread and back; verify the main and thread transcripts differ.
3. Open the multi channel. Create two conversations and verify distinct persisted
   session IDs. Refresh and switch between them.
4. Sign in as Bob. Attempt Alice's session, event, file and approval routes using
   Bob's cookie/runtime token; verify denial and unchanged persisted state.
   Try a sibling channel token and management API routes; verify denial.
5. Upload a file, ask the Agent to edit it and verify content. Try the private
   paths and a read-only file mutation; verify denial and unchanged content.
6. With a live single channel, remove effective coordinator capability (including
   inherited harness configuration). Verify rejection and read back unchanged
   configuration. Switch to multi mode, retry, and verify success at the same URL.
7. Replay an OIDC callback/handoff and submit a mutation from a sibling origin;
   verify denial. Sign out, verify the server-side credential is removed and an
   existing stream cannot admit another request after its reconnect deadline.

## Expected Result

One configured interface per channel URL, stable main ownership, independent
multi-chat sessions, capability validation and existing workspace/approval gates.
Consumers cannot access another consumer or console authority. No provider
credential appears in browser storage, URLs or rendered page data.

This case specifies acceptance coverage; its presence does not claim a passing
production run. Rendering fixtures exercise presentation separately.
