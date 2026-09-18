---
title: Foreman
description: A classifier supervising a coding agent it never has to stop.
---

[Browse the complete example](https://github.com/everruns/everruns/tree/main/examples/foreman-agent).

A fast classifier watching a slow coding agent, and a policy in ordinary Rust
deciding what to do about the numbers. A Framework port of
[thruwire/foreman](https://github.com/thruwire/foreman), which placed
[TypeSafe's Jev](https://docs.typesafe.ai/introduction) above a Codex worker and
asked whether semantic supervision can run *while* the work happens.

![Foreman terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/foreman-agent/demo/demo.gif)

## What you learn

How to run a worker session and observe it at the same time: a
[`Classifier`](/framework/examples/) turning bounded evidence into nine
probabilities in one request, and a deterministic policy that owns every
threshold, every limit, and the closed vocabulary of things the supervisor may
do.

## The two loops

The worker keeps its own reason/act loop inside a session. `session.send`
returns a receipt immediately, so the supervisory loop reads `session.events()`
beside the live turn: routine output is debounced to a floor, and every
lifecycle boundary — a tool finishing, a turn ending — takes a reading straight
away. Nothing stops for the factory to think.

## What it assesses

Five questions describe the job (`implementation_complete`, `tests_sufficient`,
`requirements_satisfied`, `needs_verification`, `ready_to_finish`) and four
describe the floor right now (`meaningful_progress`, `worker_stuck`,
`work_off_track`, `needs_human`). Each is a Noul — the probability that a
yes/no statement is true — and all nine ride one request, because questions in
a classification are answered independently and in parallel.

## What it may do about them

The classifier only estimates. The policy decides, safety and hard limits
before productivity: escalate when a person is needed or the iteration ceiling
is reached, stop a worker that is off track or stuck, retry once after a stop,
finish when the completion thresholds hold and verification is resolved, start
one independent verifier when a check is warranted, otherwise start or continue
work. Thresholds and limits are Foreman's defaults and are overridable through
`FOREMAN_*` environment variables.

## What it looks at

Never the repository. One bounded snapshot per reading: worker status, elapsed
time, tool calls and output tails, `git status`, a bounded `git diff`, the
untracked paths a diff cannot show, recent session events, verification
results, and the previous assessment and decision. An unbounded observation
would make supervision as slow as the work it is watching.

## Run it

Clone the repository and run from its root:

```bash
cargo run -p everruns-foreman-agent
```

Supervision is the cheap half, so the two halves go live separately:

| Command | Worker | Foreman | Needs |
| --- | --- | --- | --- |
| `cargo run -p everruns-foreman-agent` | scripted | deterministic | nothing |
| `… -- --live-foreman` | scripted | `jev-latest` | `TYPESAFE_API_KEY` |
| `… -- --live` | `gpt-5.6-terra` | `jev-latest` | both, with credits |

`--live-foreman` puts a real classifier over a deterministic worker, so the
numbers are a live reading of a run that goes the same way every time. That is
the mode the recording above uses.

```bash
cargo run -p everruns-foreman-agent -- --live --job "Add rate limiting, and test it."
cargo run -p everruns-foreman-agent -- /tmp/shipkit   # keep the workspace
```

The default workspace is temporary and removed on exit. Never pass a repository
you care about: the example materializes its fixture into the target and the
worker may change anything inside it.

## The floor

The bundled fixture is a small Python project that prices every parcel at one
flat rate; the job is to replace that with weight tiers and cover the
boundaries. The coding worker mounts it read-write through the [Bashkit
Shell](/capabilities/bashkit-shell/). The verifier mounts the same directory
under the default read-only policy, so "independent check" is a property of the
mount rather than a request in a prompt. The supervisor runs `git` itself
rather than asking the worker what it did.

## What the Framework changes

The architecture is Foreman's; the runtime underneath it is not. A worker is a
session rather than a subprocess, so stopping one is a cooperative turn
cancellation instead of a signal to a process group. Evidence is the canonical
event stream rather than parsed JSONL. A retry is a fresh session over the same
workspace. Read-only verification is a workspace policy. Steering exists —
sending into a live turn applies at the next iteration boundary — and is
deliberately left out of the policy's vocabulary so the comparison with the
original stays honest.

## Limits

This is an architectural experiment, and porting it does not make it a proven
one. Classifier accuracy for this use is unproven and the thresholds are
uncalibrated: false positives stop useful workers, false negatives let bad work
continue. Observations are bounded and therefore incomplete. One coding worker
runs at a time, a verifier reports evidence rather than proof, and the session
state is in memory.
