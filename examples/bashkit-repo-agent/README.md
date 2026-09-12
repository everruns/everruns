# Bashkit Repo Agent

Cut a release in a bundled repository through the sandboxed
[Bashkit](https://bashkit.sh) shell, then verify the claimed changes on disk.
This is a real `gpt-5.6-terra` agent, not a scripted turn.

![Bashkit Repo Agent terminal demo](demo.gif)

## What you learn

Mounting a scoped read-write workspace, giving an agent one execution
capability, recovering from an unsupported command, and checking the resulting
state independently of the model's final answer.

## Scenario and expected outcome

The bundled `sample-repo/` is a two-crate Cargo workspace with changelog
fragments. The agent must cut release `0.2.0`: update both manifests and the
path-dependency pin, create a dated changelog section, retain the previous
release, and remove the folded fragments. The host then re-reads the working
copy and fails the process if any invariant is false.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. This folder is
self-contained **within the workspace**: its manifest references local
Framework crates, so copying the folder alone is not sufficient.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export OPENAI_API_KEY="your-key"
cargo run -p everruns-bashkit-repo-agent
```

With no argument, the working copy uses a temporary directory removed on exit.
Pass a scratch directory after `--` to inspect it afterwards:

```bash
cargo run -p everruns-bashkit-repo-agent -- /tmp/release-run
```

The configured model is `gpt-5.6-terra`. A funded OpenAI account is required;
live runs can incur charges. Never point this example at a repository you care
about: the agent may change anything inside the mounted workspace.

## Build the agent

The editable system prompt lives in `instructions.md`. The workspace is a real
host directory, but the policy and Bashkit runtime clamp agent access to its
mounted `/workspace` tree.

```rust
let agent = Agent::builder()
    .name("bashkit-repo-agent")
    .instructions(include_str!("../instructions.md"))
    .provider(provider)
    .model(MODEL)
    .workspace(workspace)
    .workspace_policy(WorkspacePolicy::read_write())
    .capability(BashkitShell::new())
    .build()?;
```

## Run and verify

`demo::run` subscribes before sending, shows a bounded shell timeline, and
waits for a successful turn. The final answer is not treated as proof: `verify`
checks the manifests, changelog, and fragment directory from the host.

```rust
let session = Engine::new().create(agent);
demo::run(&session, &release_request(&release_date)).await?;

let checks = verify(&root, &release_date);
if checks.iter().any(|check| !check.passed) {
    return Err("release verification failed".into());
}
```

Use `session.send_and_wait(request).await?` instead when you do not need the
live tool timeline.

## How the capability works

Bashkit interprets bash in-process against the session filesystem: no
`/bin/bash`, subprocess, host path outside `/workspace`, network, or `git`.
Commands, loops, and script size are bounded. The recording includes a genuine
recovery: `find -delete` is unsupported, so the agent inspects partial state and
uses the portable `-exec rm -f {} \;` form.

## Validate the behavior

```bash
cargo test -p everruns-bashkit-repo-agent
python3 examples/bashkit-repo-agent/render_demo.py --check
```

Tests validate agent construction, the fixture's starting state, and the
host-side assertions without contacting OpenAI. CI does not grade live model
quality.

## Demo and recording

`demo.txt` is output from a successful live run. `render_demo.py` automatically
creates readable pages and durations; VHS replays them without another API
call.

```bash
cd examples/bashkit-repo-agent
bash record.sh
```

Recording needs Python 3, VHS, ffmpeg, a VHS-compatible browser, and funded
OpenAI credentials. The script preserves the previous successful transcript if
the provider run fails. Inspect recordings before sharing when adapting this
example to private repositories.

## Adapt it

Replace `sample-repo/`, `release_request`, and `verify` with a disposable fixture
and independent assertions for your workflow. Keep the mounted root narrow,
start read-only unless mutation is required, and validate important claims from
host state after the turn.

## Boundaries

This demonstrates one sandboxed repository mutation, not a general coding
agent. It has no network or Git credentials and cannot create commits or push.
The session itself is in-memory.

## Source map

`src/main.rs`: agent, run, and verification; `src/sample_repo.rs`: fixture
materialization; `sample-repo/`: input repository; `instructions.md`: agent
instructions. `examples/demo-support::shell` handles terminal presentation;
`record.sh` and `render_demo.py` handle recording.

## See also

- [Bashkit Shell capability](https://docs.everruns.com/capabilities/bashkit-shell/)
- [`coding-cli`](../coding-cli) — a full terminal coding agent combining
  Bashkit with file, search, and skill capabilities
