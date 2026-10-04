# Foreman agent

A coding worker edits a repository while a cheap supervisor watches it. The
supervisor asks nine questions in one request; ordinary Rust decides whether to
continue, stop, retry, verify, finish, or ask for a person.

This ports the architecture of [thruwire/foreman](https://github.com/thruwire/foreman)
and its [theory of semantic supervision](https://github.com/thruwire/foreman/blob/main/docs/theory.md).

![Enter a job, dispatch coding, monitor live work, verify, and run tests](demo/demo.gif)

## Run the demo

From the Everruns repository root, with Rust/Cargo, Git, and a running local
Docker daemon. The demo makes a temporary shell repository under your home directory (shared by
Docker on macOS), then runs a **real**
coding model and **real** supervisor. It needs funded provider credentials:

| Variable | Used by |
| --- | --- |
| `TYPESAFE_API_KEY` | Supervisor: TypeSafe `jev-latest` |
| `OPENROUTER_API_KEY` | Worker and verifier: `meta/muse-spark-1.3-contributor` |

Load credentials through Doppler:

```bash
doppler run --project everruns-dev --config dev -- \
  cargo run -p everruns-foreman-agent --bin foreman -- demo --interactive
```

Already exported those keys? Run the same Cargo command without `doppler run`.
Enter a job for the bundled shipping repository. The demo supplies the four
weight tiers as fixed requirements. Omit `--interactive` to use its default job.
The recorded run shows:

1. Foreman starts; you enter the job.
2. `START_WORKER`: Rust policy dispatches a coding worker, which edits real files.
3. Supervisor readings marked **RUNNING**, taken during the worker's turn.
4. `START_VERIFIER`: policy dispatches a second session with a read-only mount.
5. `FINISH`, followed by fixed acceptance checks and an actual run of the suite.

The worker backend is configured by `--worker`; Foreman selects when each role
runs, using classifier scores and deterministic policy.

The demo exits unsuccessfully if supervision does not finish or the final checks
fail. Its temporary repository is removed on exit. To keep the result, use
`demo --repo /tmp/foreman-demo` with an empty scratch directory.

## Run against your repository

Install the binary from this workspace:

```bash
cargo install --path examples/foreman-agent --locked
```

Then use Foreman's invocation:

```bash
doppler run -- foreman run --repo ./my-project \
  --job "Add rate limiting to the API and make sure it is properly tested." \
  --worker codex --tests "cargo test --offline"
```

The worker modifies the named repository. `run` never installs the demo fixture.

| Worker | Invocation | Credentials |
| --- | --- | --- |
| `session` (default) | Framework session using Bashkit | `OPENROUTER_API_KEY` |
| `codex` | `codex exec --cd {repo} --sandbox workspace-write --color never --json {mission}` | Codex login or `OPENAI_API_KEY` |
| `yolop` | `yolop -C {repo} -p {mission}` | yolop credential store or `ANTHROPIC_API_KEY` |

Use an external worker for repositories needing a compiler, Python, Node, Git,
or network access. The default Bashkit worker only interprets shell commands
and accesses its workspace mount; it cannot launch native programs.

Every backend uses the same Framework verifier under `WorkspacePolicy::read_only()`.
External workers therefore also need `OPENROUTER_API_KEY` for verification and
`TYPESAFE_API_KEY` for supervision. The verifier can inspect arbitrary code,
but can execute only Bashkit-supported shell tests.

An operator-supplied command is also supported:

```bash
foreman run --repo . --job "Add boundary tests" \
  --worker-command "mycli --cd {repo} --task {mission}"
```

Templates are whitespace-separated argv entries. `{repo}` and `{mission}` must
be whole entries; their values are passed directly to the process, without a
shell. Quoted literal arguments are not parsed. Known workers inherit only
ordinary process settings and their listed provider key. Custom commands must
use their own credential store. The supervisor key is never forwarded.

## Read the supervision loop

Start with `src/factory.rs`, then `src/foreman.rs` and `src/policy.rs`.

```text
coding session / external process         supervisor
  reason → tools → edits → tests            bounded observation
              │                                    ↓
              └──── events / output ──────── nine probabilities
                                                   ↓
                                            deterministic policy
                                                   ↓
                                     continue / stop / retry / verify / finish
```

`session.send()` starts the worker without waiting for its turn to finish.
`session.events()` supplies typed evidence while the supervisor runs beside it.
Activity is debounced; quiet work still receives periodic readings. Stopping a
session uses `TurnHandle::cancel()`; external workers are stopped as a process
group on Unix.

Each reading carries worker status, output tails, Git status and diff, untracked
paths, recent events, test/verification results, and the previous assessment and
decision. Defaults limit diff text to 20,000 characters, each output tail to
12,000 characters, events to 30, and worker history to 10.

`src/foreman.rs` preserves the original nine questions: implementation complete,
tests sufficient, requirements satisfied, needs verification, ready to finish,
meaningful progress, worker stuck, work off track, and needs human. All nine use
one `Decisions` request, the Framework's current decision API. It returns only
probabilities; `src/policy.rs` owns the actions and thresholds.

The policy prioritizes human need and limits, then stopping and one retry,
verification, completion, and continued work. **FINISH requires a completed
independent verification pass. A configured failing test run blocks FINISH.**

## Independent tests and limits

With `--tests`, the supervisor runs the command before coding and again when
workers are idle. It records exit status, output, and result age; worker claims
about green tests cannot replace that result.

Tests run in a networkless Docker container with a read-only source mount and a
writable disposable copy. The default image contains Rust and Bash; dependencies
must already be available offline. Use `--test-image <image>` for another local
toolchain, such as a project image containing Node and installed dependencies.
Docker receives its connection settings, but the test container receives no
host credentials.

The supervisor sends the bounded evidence to TypeSafe. Repository text and tool
output remain untrusted input. Scores are uncertain and thresholds uncalibrated;
a passing suite establishes only that the tests that exist passed. Sessions and
supervision state are in memory. External workers use their own permission model.

## Tests and recording

```bash
cargo test -p everruns-foreman-agent
bash examples/foreman-agent/demo/record.sh --check
```

Offline tests substitute `DecisionsService` and scripted models. They cover
readings during live turns, the complete coding → verifier → FINISH path,
stop/retry/escalation, unavailable supervision, and external process failures.
All test scaffolding lives in `tests/`.

With VHS, ffmpeg, ttyd, a compatible browser, Docker, and the credentials above:

```bash
bash examples/foreman-agent/demo/record.sh
```

The script records the real binary through VHS. It publishes the GIF and
[transcript](demo/transcript.txt) only if the run includes job entry, coding
dispatch, live readings, read-only verification, FINISH, passing acceptance checks, and a successful final
suite. It also checks the binary's exit code.

| Source | Responsibility |
| --- | --- |
| `src/main.rs`, `src/cli.rs` | Credentials, arguments, and entry points |
| `src/factory.rs` | Observation loop and interventions |
| `src/foreman.rs`, `src/policy.rs` | Nine questions and deterministic policy |
| `src/worker.rs`, `src/agent.rs` | Worker backends and read-only verifier |
| `src/observation.rs` | Bounded evidence and isolated test execution |
| `src/run.rs`, `src/terminal.rs` | Terminal presentation |
| `src/fixture.rs`, `src/resources/` | Demo repository, prompts, acceptance checks |

See [Bashkit Repo Agent](../bashkit-repo-agent/) for a smaller example of one
agent editing a mounted repository.
