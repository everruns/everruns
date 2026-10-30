---
type: Test Case
title: "TC002: Cloudflare Agent registration and trace"
description: Verify self-service HTTPS registration and a real private-D1 Agent through the ordinary Playground and Trace.
tags:
  - everruns
  - test-case
  - ui
  - custom-workers
---

# TC002: Cloudflare Agent registration and trace

## Description

A user connects a customer-hosted Cloudflare Agent without database access. The
ordinary Agent, Playground, session and Trace surfaces remain its product identity.

## Preconditions

- A DB-backed local stack with external agents enabled and encryption configured.
- A disposable deployed Worker and seeded private D1 database from the
  [example](../../../../examples/cloudflare-data-agent/README.md).
- An org owner with permission to change dangerous Agent settings.
- The endpoint secret available privately; never capture it in screenshots.

## Steps

1. Create a normal Agent. Open Configure execution on its Agent page.
2. Enter the HTTPS endpoint without its trailing slash, plus an incorrect secret.
   Verify ownership fails and no binding appears.
3. Enter the correct endpoint secret and connect. Verify the secret field clears,
   the normalized endpoint appears, and the Execution badge says External worker.
4. Open Test in Playground. Ask for north sales, exact cents and order count.
   Verify the reply reports 27278 cents and two orders.
5. Open the session Trace. Verify the actual model generation, sales_summary
   arguments/result and reported usage appear under the canonical turn.
6. Send a follow-up asking which region was discussed. Verify north is remembered.
7. Disable the external worker. Verify Disabled appears and another user message
   remains queued rather than running the built-in runtime.
8. Reconnect with a rotated secret after installing it on the Worker. Verify the
   queued input completes through that endpoint.
9. Verify an ordinary org member cannot change execution credentials.

## Expected Result

Registration requires ownership and permission; no operator SQL or infrastructure
credential reaches the Worker. Normal chat, history, SSE, Trace and work projection
use canonical CP events. Revocation retains routing exclusion.

## Cleanup

Disable the binding, stop the local stack, and remove the disposable Cloudflare
Worker and D1 database. Keep only non-secret test evidence.
