---
type: Test Case
title: "TC009: Scheduled activity crash and retry boundaries"
description: "Verify provider-free schedule dispatch and trigger invocation remain atomic across failed acknowledgements and retries."
tags:
  - everruns
  - test-case
  - api
  - scheduled-tasks
---
# TC009: Scheduled activity crash and retry boundaries

## Preconditions

Use a disposable PostgreSQL database and a recording runner, not a live LLM provider.
Use both ephemeral and shared schedule session modes. Do not inject failures in production.

## Steps and expected results

1. Fail activity dispatch immediately before commit. No task or execution remains,
   and the due time is unchanged. Retry creates exactly one task.
2. Commit activity dispatch, let a worker claim the task, and replay the original
   occurrence. No second task is created. Repeat with concurrent scheduler calls.
3. Invoke a schedule task, commit its result, then simulate a lost acknowledgement
   by invoking the same task again. The same result is returned; no second input,
   delivery or ephemeral session appears.
4. Fail invocation before its transaction commits. Session, input, delivery and
   receipt all roll back. Concurrent retries produce one committed dispatch.
5. Invoke a different task for the same trigger. It executes normally; shared mode
   retains its session while ephemeral mode creates a new one.
6. Reuse a receipt with a different task identity. Reject it without exposing the
   saved result or starting work.
7. For a channel target, fail before commit. Neither the recovery outbox row nor
   its background sender survives. On commit, recovery sees the row even if the
   process dies before the background sender starts.

## Automated coverage

- Durable store conformance tests cover both memory and PostgreSQL dispatch replay.
- PostgreSQL failpoint tests cover precommit rollback.
- Agent-trigger command tests cover invocation replay, rollback and concurrency.
- Channel host tests cover pending persistence and postcommit delivery spawning.

These guarantees cover dispatch, not exactly-once downstream tools or external
platform delivery. Generic workflow schedules and manual invocations are distinct.
