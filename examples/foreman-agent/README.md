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
immediately, and a child process is simply left running; either way the
supervisor reads the evidence beside the live work. Activity marks the run
dirty and waits for the debounce floor, and only a worker finishing bypasses
it — forcing a reading on every tool call spends the whole iteration budget on
a chatty worker before it finishes, which a live run showed plainly.

`readings_land_while_the_worker_is_still_working` in `src/factory.rs` is the
test that holds this: it counts readings taken while a worker's turn is
unresolved, and fails if supervision waits its turn.

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

Foreman's own two entry points, and they mean the same things here:

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
cargo run -p everruns-foreman-agent --bin foreman -- demo
```

`demo` walks the whole runtime over a disposable fixture with nothing to pay
for. `run` supervises real work in a repository you name:

```bash
foreman run --repo ./my-project --job "Add rate limiting, and test it."
```

Supervision is the cheap half, which is the premise, so the two halves go live
separately:

| Command | Worker | Foreman | Needs |
| --- | --- | --- | --- |
| `foreman demo` | scripted | a stub service, from a table | nothing |
| `foreman demo --live-foreman` | scripted | `jev-latest` | `TYPESAFE_API_KEY` |
| `foreman run …` | your choice, below | `jev-latest` | `TYPESAFE_API_KEY` + the worker's |

`--live-foreman` puts a vendor's classifier over a deterministic worker, so the
numbers on screen are a live reading of a run that goes the same way every
time. That is the mode the demo above records.

`demo` writes its fixture into a temporary directory unless `--repo` says
otherwise, and `run` never writes a fixture at all — `--repo` is your project,
and the only thing that touches it is the worker. It will be modified.

## Who does the work

`--worker` picks the crew. All three are watched identically, because what the
supervisor reads is a bounded observation and the strongest evidence in one —
the repository's own diff — is gathered by the host either way.

| `--worker` | What runs | Independent verification |
| --- | --- | --- |
| `session` (default) | An Everruns session on the Bashkit shell, `meta/muse-spark-1.3-contributor` through OpenRouter | A second session under the default read-only workspace policy |
| `codex` | `codex exec --cd … --sandbox workspace-write --color never --json …`, the line Foreman itself runs | The same CLI with `--sandbox read-only` |
| `yolop` | `yolop -C … -p …`, its one-shot print interface | Mission only — yolop publishes no read-only mode |

Anything else is a template:

```bash
foreman run --repo . --job "…" --worker-command "mycli --cd {repo} --task {mission}"
```

`{repo}` and `{mission}` are substituted as whole arguments, so no shell sees
either: a mission carrying quotes, newlines, or a semicolon is still one argv
entry.

A session is observed through its own canonical event stream, which arrives
already typed — tool calls separated from model steps from output. A CLI offers
none of that, so an external worker is observed through stdout and stderr, and
a JSONL line that names its own `type` (as Codex's `--json` does) counts as a
step. Neither Codex nor yolop is installed in CI and neither needs to be: the
tests drive the same path with a stand-in child process.

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

## One supervisor

There is one, and it is always real: `src/foreman.rs` holds a `Classifier` and
a budget, builds one request, and parses nine answers. A run with no
credentials is not a second supervisor with fabricated numbers — it is the same
code over a different
[`ClassifierService`](https://docs.rs/everruns/latest/everruns/trait.ClassifierService.html),
which is the Framework's own seam for answering typed questions without a
vendor:

```rust
#[async_trait]
impl ClassifierService for Rehearsed {
    async fn evaluate(&self, request: ClassificationRequest)
        -> Result<ClassificationOutcome, AgentLoopError>
    {
        let reading = &self.readings[Self::phase(&request.state)];
        // one Noul per question id
    }
}
```

The stub receives the observation as JSON, exactly as a vendor's service does,
and answers from `src/resources/demo/readings.json` — a table keyed by what is
on the floor, beside the shell scripts the scripted worker runs. So the demo
exercises the whole request path rather than bypassing it, and the numbers it
answers with are data you can edit without touching Rust.

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
the nine-question round trip against a simulated classifier, both worker
backends' command lines, and six whole runs through the real runtime: readings
landing mid-turn, a stuck worker stopped and retried once and escalated, a
supervisor that cannot answer, an external worker watched while it streams, a
missing external binary failing by name, and an external worker killed when the
policy stops it. CI does not grade a live model's code, so a live run remains
the behavioral proof.

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
- Foreman's own worker — the Codex CLI — is still one of the options here, so
  the two runtimes can be pointed at the same job and compared.

## Limits

This is an architectural experiment, and porting it does not make it a proven
one. Classifier accuracy for this use is unproven and the thresholds are
uncalibrated: false positives stop useful workers, false negatives let bad work
continue. Observations are bounded and therefore incomplete. One coding worker
runs at a time. A verifier reports evidence, not proof. State lives in memory —
the Framework's durable session store is a separate example. Bashkit is a
sandbox for the shell, not a safe harness for an untrusted repository.

## Source map

`src/cli.rs`: the command line; `src/main.rs`: wiring and the host's own
report; `src/factory.rs`: the two loops, the state, and the interventions;
`src/worker.rs`: both crews and the evidence pumps; `src/foreman.rs`: the nine
questions and the classifier call; `src/policy.rs`: thresholds, limits, and the
decision; `src/observation.rs`: the bounded snapshot; `src/agent.rs`: the two
agents; `src/demo_run.rs`: the scripted worker and the stub classifier service;
`src/terminal.rs`: layout only, over `everruns-example-demo::style`;
`src/fixture.rs` and `src/resources/`: the repository and the prompts;
`demo/`: the recording.

## See also

- [thruwire/foreman](https://github.com/thruwire/foreman) — the original, in
  Python over the Codex CLI, and its
  [theory of semantic supervision](https://github.com/thruwire/foreman/blob/main/docs/theory.md)
- [Classification](https://docs.everruns.com/framework/examples/) — asking for
  numbers instead of prose
- [`bashkit-repo-agent`](../bashkit-repo-agent) — one supervised worker, without
  the supervisor
