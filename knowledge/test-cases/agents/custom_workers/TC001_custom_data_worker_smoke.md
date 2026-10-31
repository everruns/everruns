---
type: Test Case
title: "TC001: Custom workers - Private data agent smoke"
description: Verify a real-model custom worker uses private tools while the control plane retains intake, event delivery, routing and credential authority.
tags:
  - everruns
  - test-case
  - agents
  - custom-workers
---

# TC001: Custom workers - Private data agent smoke

## Description

Verify a real-model custom worker uses private tools while the control plane
retains intake, event delivery, routing and credential authority.

## Preconditions

- Disposable self-hosted stack, with PostgreSQL, NATS, CP and a live built-in worker.
- Local `AUTH_MODE=none` only in development, or an authenticated operator with
  agent/session management permission and the explicit org selected.
- Restricted custom listener enabled, with verified TLS for the container path.
- Operator CP database access for provisioning, separate from the worker's private
  database. The worker receives neither operator API nor CP database credentials.
- Real model credential and matching worker/server source revision.
- For container qualification, the example image and a public server CA. No CA
  private key, CP private key or internal fleet token is mounted into the worker.

## Test Data

| Field | Value |
| --- | --- |
| Fixture | A newly created agent, Playground session and deployment per run |
| Initial north orders | 18,037 and 9,241 cents; two orders, 27,278 cents |
| Updated north orders | Add 1,234 cents; three orders, 28,512 cents |
| Tool | `sales_summary` with a bound region parameter and read-only SQLite access |
| Runtime | Framework, real OpenAI model; no simulator |

## Steps

1. Run the [checked-in smoke runner](../../../../examples/custom-data-agent/smoke.py)
   against the stack, first as a process and then with `--image` and verified TLS.
   Commands and certificate prerequisites are in the
   [example README](../../../../examples/custom-data-agent/README.md#full-smoke-test).
2. Save the newly created fixture IDs from `session.json`. Subscribe from sequence
   zero before sending input so a historical result cannot satisfy this run.
3. While the custom worker is absent, submit the first question and confirm no
   built-in worker starts its turn.
4. Start the real worker. Within the runner's deadline, require a model generation,
   successful database tool result, correct final answer, ordinary messages/work
   responses, trace correlation, live SSE output and an idle final status.
5. Replace the worker process/container, update its private database and ask for
   fresh data in the same session. Require the changed aggregate and CP history.
6. Stop the worker and revoke its credential. Submit new input, start the revoked
   worker and require authentication failure, unchanged input/execution state and
   no built-in fallback.
7. Rotate the credential on the same deployment and start a fresh replica. Require
   the original queued input to finish, without resubmission or duplicate terminal
   events. Confirm durable sequences are unique and ordered.
8. For the container path, inspect the actual user, capabilities, resource limits,
   mounts, network mode and environment names. Require no privileged/host-network
   execution, writable host mount or CP credential variable.
9. Confirm cleanup stopped the custom worker and disabled its credential. Retain
   the isolated session and private JSON evidence for UI inspection.

## Expected Result

Both modes report `status: passed`, three completed turns, canonical event and SSE
evidence, and matching private database aggregates. Container evidence records the
actual image digest and runtime boundary. The smoke runner fails on missing events,
incorrect totals, premature worker exit or a deadline, and revokes its credential
on failure. Compilation, fixture-only tests or an ignored live test do not count
as an end-to-end pass.
