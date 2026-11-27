---
type: Specification
title: Full-org workflow checks on data plane v2
description: "The nightly check that a real org's everyday workflows work on v2 end to end with a real model: what it runs, how it passes, and the rows still to add."
tags:
  - execution
  - data-plane-v2
  - testing
---

# Full-org workflow checks on data plane v2

The milestone has two halves: 75 000 agents (the verifier,
`verifiers/agents-75k`) and a full org working end to end on v2. This check
is the second half. Until it existed, v2 workflow bugs (a thread that could
not call `finish` or `ask_parent`) were found by a person, not a test.

## What runs

[`scripts/e2e-v2-workflows.mjs`](../../../scripts/e2e-v2-workflows.mjs), from
[`.github/workflows/v2-workflows.yml`](../../../.github/workflows/v2-workflows.yml):
nightly, by hand, and on a pull request that changes the suite.

- **The Compose stack** (control plane plus agentd, v2 by default) on a
  GitHub runner, from one commit's images: main's own (a missing set is
  built by an amd64-only Docker Publish main snapshot), or a pull request's.
- **A real model**, GPT-6 Luna through OpenRouter, from Doppler. Workflows
  depend on the model choosing tools, which a simulated model only pretends.
  The key reaches the server through startup seeding only.
- **Driven like a user**, through the public API.
- **One pass or fail per row** in the run summary, and a JSON report.

## Rules

- A row passes on what events and records show (tool calls that succeeded,
  task states, the session's git diff, stream event types), never on the
  reply's wording.
- A failed row gets one retry; both attempts are reported.
- No skipped rows: a row that cannot run reports a failure.
- The last row checks the database: the org is placed on the data plane and
  the server's own `sessions` table is empty, so every row ran on v2.

## Rows

| # | Workflow | Passes when | State |
|---|---|---|---|
| 1 | Chat | the reply streams deltas; `wait=true` returns it; cancel ends a turn as cancelled | in the suite |
| 2 | Coding | the agent writes and edits files, runs bash, and the session's git diff lists the edit | in the suite (session files and the in-process shell; the repositories Changes panel needs a sandbox, later) |
| 3 | Threads | `start_thread` runs a child that asks its parent with `ask_parent`, gets the answer, calls `finish`; the assignment succeeds | in the suite |
| 4 | Spawned agents | the parent spawns another agent, which runs and reports back | to add |
| 5 | Schedules and triggers | a schedule fires on time; a saved-script trigger runs | to add |
| 6 | MCP | an org MCP server and a person's own connection both serve a tool call | to add |
| 7 | Knowledge, memory, SQL | a knowledge page is read, a fact remembered across sessions, a session database queried | to add |
| 8 | Approvals | a guarded tool waits, runs once approved; auto-approval skips the wait | to add |
| 9 | Spend | usage reaches the control plane; a session at zero balance is refused | to add |
| 10 | Slack | a mention in the test workspace gets a threaded reply (Socket Mode) | to add |
| 11 | Moves | a v1 session moves to v2 and continues | to add |
| 12 | Console | chat, the Threads panel and the Files tab render a v2 session | to add |

Done means every row passes on main for a week; then the v1, v2 or hybrid
decision.

Thread behaviour the suite cannot force with a real model is covered in the
testkit (`crates/agentd-testkit/tests/subagents.rs`): a thread is offered no
thread tools, and a thread turn that ends without `finish` flags its
parent's assignment (`crates/agentd/src/thread_turns.rs`).
