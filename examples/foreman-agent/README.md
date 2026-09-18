# Foreman

A fast classifier watching a slow coding agent, and a policy in ordinary Rust
deciding what to do about the numbers. A Framework port of
[thruwire/foreman](https://github.com/thruwire/foreman), which put
[TypeSafe's Jev](https://docs.typesafe.ai/introduction) above a Codex worker and
asked whether semantic supervision can run *while* the work happens.

![Foreman terminal demo](demo/demo.gif)

**Generative models work. Foreman watches the work.**

## What you learn

How to run a worker session and observe it at the same time: a
[`Classifier`](https://docs.rs/everruns/latest/everruns/classifier/) turning
bounded evidence into nine probabilities in one request, and a deterministic
policy that owns every threshold, every limit, and the closed vocabulary of
things the supervisor may do.

## The two loops

```text
WORKER SESSION                            SUPERVISORY LOOP
reason                                    observe
  │                                         │
  ▼                                         ▼
tool                                      assess
  │                                         │
  ▼                                         ▼
observe ──── canonical events ───────────► Classifier · 9 nouls, 1 request
  │                                         │
  ▼                                         ▼
edit                                      decide (Rust policy)
  │                                         │
  ▼                                         ▼
check ◄───── cancel / start / finish ───── intervene
```

The worker never stops to be watched. `session.send` returns a receipt
immediately; the supervisor reads `session.events()` beside the live turn.
Output and tool calls make the run dirty and respect the debounce floor; only
worker lifecycle boundaries — a turn ending, a verification reporting — bypass
it. Forcing a reading on every tool call spends the whole iteration budget on a
chatty worker before it finishes, which a live run showed plainly.

## What it assesses

Five questions describe the job and four describe the floor right now. Every one
is a Noul — the probability that a yes/no statement is true — and all nine ride
one request, so asking nine costs one round trip:

| The job | The floor |
| --- | --- |
| `implementation_complete` | `meaningful_progress` |
| `tests_sufficient` | `worker_stuck` |
| `requirements_satisfied` | `work_off_track` |
| `needs_verification` | `needs_human` |
| `ready_to_finish` | |

Question wording is carried over from Foreman unchanged, and lives in
`src/foreman.rs`. Wording is the interface to a classifier the way a schema is
the interface to an API: a threshold calibrated against one phrasing is not
evidence about another.

## What it may do about them

The classifier only estimates. `src/policy.rs` decides, in this order — safety
and hard limits before productivity:

`ESCALATE` on `needs_human` · `ESCALATE` at the iteration ceiling ·
`STOP_WORKER` when off track · `STOP_WORKER` when stuck · `RETRY_WORKER` once
after a stop · `FINISH` when completion thresholds hold and verification is
resolved · `START_VERIFIER` when a check is warranted · `START_WORKER` while
work remains · `CONTINUE`.

Thresholds are Foreman's defaults (human 0.80, off track 0.80, stuck 0.80,
verification 0.65, finish 0.85, requirements 0.80, tests 0.75), and the
`FOREMAN_*` environment variables in `src/policy.rs` override the intervals and
limits.

## What it looks at

Never the repository. One bounded snapshot per reading: worker status, elapsed
time, tool calls and output tails, `git status`, a bounded `git diff`, the
untracked paths a diff cannot show, recent session events, verification
results, and the previous assessment and decision. Defaults are a 20,000
character diff, 12,000 characters per tail, 30 events, and 10 workers of
history. An unbounded observation would make supervision as slow as the work it
is watching.

That snapshot goes to the classifier's service on every reading, so on a live
run a bounded slice of the repository — the diff, the changed paths, whatever
the worker printed — leaves the machine. Point `--live` at a private repository
only if that is acceptable for it, the same judgment any third-party search or
model call asks for. The offline mode sends nothing anywhere.

The repository content in an observation is also untrusted input to the
classifier, so a hostile repository can try to talk it into a number. That it
can only produce a *number* is the point: the classifier never names an action,
and every action the policy can take is in `src/policy.rs` where it can be read.
The worst a poisoned reading buys is a wrong intervention on a worker that is
already sandboxed.

## Run it

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
cargo run -p everruns-foreman-agent
```

Three modes, because the two halves cost very differently:

| Command | Worker | Foreman | Needs |
| --- | --- | --- | --- |
| `cargo run -p everruns-foreman-agent` | scripted | deterministic | nothing |
| `… -- --live-foreman` | scripted | `jev-latest` | `TYPESAFE_API_KEY` |
| `… -- --live` | `meta/muse-spark-1.3-contributor` | `jev-latest` | both keys |

Supervision is the cheap half, which is the whole premise: `--live-foreman`
puts a real classifier over a deterministic worker, so the numbers on screen are
a live reading of a run that goes the same way every time. That is the mode the
demo above records. The worker's own model is Muse Spark's Contributor tier
through OpenRouter, chosen for the same reason — a worker nobody can afford to
run often is a poor subject for an experiment about watching one.

A `--live` run takes a minute or two and costs cents: the worker implements the
tiers, a read-only verifier quotes the file and line it relied on, and the
policy finishes on numbers around
`ready_to_finish 0.91 · requirements_satisfied 0.94 · tests_sufficient 0.91`.

Neither half is deterministic, so the path varies, and escalation is a real
outcome rather than a failed run: a job that leaves the rate schedule unstated
pushes `needs_human` past 0.80, and a run that spends its three workers while
`ready_to_finish` sits at 0.82 asks for a person instead of guessing. Both are
the supervisor working. The process exits nonzero only when the factory decided
nothing at all — when it ran out of clock, or finished without touching the
repository. `FOREMAN_MAX_WORKERS` and the other `FOREMAN_*` variables move the
budgets if you want a longer leash.

```bash
cargo run -p everruns-foreman-agent -- --live --job "Add rate limiting, and test it."
cargo run -p everruns-foreman-agent -- /tmp/shipkit   # keep the workspace
```

The default workspace is temporary and removed on exit. Never pass a repository
you care about: the example materializes its fixture into the target and the
worker may change anything inside it.

## The floor

`src/resources/sample-repo/` is a small Python project that prices every parcel
at one flat rate. The job is to replace that with weight tiers and cover the
boundaries. The coding worker mounts it read-write through
[Bashkit](https://bashkit.sh); the verifier mounts the same directory under the
default read-only policy, so "independent check" is a property of the mount
rather than a request in a prompt:

```rust
pub fn worker(model: Model, workspace: &Path) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("factory-worker")
        .instructions(include_str!("resources/worker.md"))
        .model(model)
        .max_iterations(16)
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_write())
        .capability(BashkitShell::new())
        .build()
}
```

The supervisor runs `git` itself rather than asking the worker what it did.

## Ask the nine questions

```rust
let mut classification = classifier.about(serde_json::to_value(observation)?);
for dimension in &DIMENSIONS {
    classification = classification.noul(dimension.id, dimension.question);
}
let answers = classification.send().await?;
```

Ids label answers for your code and never reach the model, so every question has
to read on its own.

## Validate it

```bash
cargo test -p everruns-foreman-agent
bash examples/foreman-agent/demo/record.sh --check
```

The suite is offline. It covers every policy branch, the observation bounds,
the nine-question round trip against a simulated classifier, and two whole runs
through the real runtime: one that finishes after verifying its own work, and
one where a stuck worker is stopped, retried once, and escalated. CI does not
grade a live model's code, so a live run remains the behavioral proof.

## Demo and recording

With VHS, ffmpeg, a VHS-compatible browser, and a TypeSafe key:

```bash
bash examples/foreman-agent/demo/record.sh
```

It uses `TYPESAFE_API_KEY` when exported, otherwise Doppler project
`everruns-dev`, config `dev`, and replaces the checked-in GIF and transcript
only after a run that actually reached a decision. `demo/transcript.txt` is the
same run as static text.

## What the Framework changes

The architecture is Foreman's; the runtime underneath it is not.

- A worker is a session, not a subprocess. Stopping one is
  `TurnHandle::cancel`, which resolves the turn as cancelled and leaves the
  session's own history consistent — there is no process group to signal.
- Evidence is the session's canonical event stream rather than parsed JSONL, so
  tool calls, model steps, and output arrive already typed.
- A retry is a fresh session over the same workspace.
- Read-only verification is a workspace policy rather than an instruction.
- Steering exists (`session.send` during a live turn applies at the next
  iteration boundary) and is deliberately **not** in the policy's vocabulary.
  V1 keeps Foreman's seven actions so the comparison stays honest; a nudge is
  the obvious next experiment.

## Limits

This is an architectural experiment, and porting it does not make it a proven
one. Classifier accuracy for this use is unproven and the thresholds are
uncalibrated: false positives stop useful workers, false negatives let bad work
continue. Observations are bounded and therefore incomplete. One coding worker
runs at a time. A verifier reports evidence, not proof. State lives in memory —
the Framework's durable session store is a separate example. Bashkit is a
sandbox for the shell, not a safe harness for an untrusted repository.

## Source map

`src/main.rs`: modes, wiring, and the host's own report; `src/factory.rs`: the
two loops, the event pump, and the interventions; `src/foreman.rs`: the nine
questions and the classifier call; `src/policy.rs`: thresholds, limits, and the
decision; `src/observation.rs`: the bounded snapshot; `src/agent.rs`: the two
agents; `src/terminal.rs`: presentation only; `src/fixture.rs` and
`src/resources/`: the repository and the prompts; `demo/`: the recording.

## See also

- [thruwire/foreman](https://github.com/thruwire/foreman) — the original, in
  Python over the Codex CLI, and its
  [theory of semantic supervision](https://github.com/thruwire/foreman/blob/main/docs/theory.md)
- [Classification](https://docs.everruns.com/framework/examples/) — asking for
  numbers instead of prose
- [`bashkit-repo-agent`](../bashkit-repo-agent) — one supervised worker, without
  the supervisor
