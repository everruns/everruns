---
title: Bashkit Repo Agent
description: Cut and verify a release in a real repository using a sandboxed shell.
---

The [Bashkit Repo Agent source](https://github.com/everruns/everruns/tree/main/examples/bashkit-repo-agent)
cuts a release in a bundled repository through the sandboxed [Bashkit
Shell](/capabilities/bashkit-shell/), then verifies the claimed changes on disk.
This is a real `gpt-5.6-terra` agent, not a scripted turn.

![Bashkit Repo Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/bashkit-repo-agent/demo.gif)

## What you learn

Mounting a scoped read-write workspace, exposing one execution capability,
recovering from an unsupported command, and checking resulting state
independently of the model's final answer.

## Scenario and expected outcome

The bundled fixture is a two-crate Cargo workspace with unreleased changelog
fragments. The agent must cut `0.2.0`: update both package versions and their
path-dependency pin, create a dated changelog section, preserve the previous
release, and remove the folded fragments. The host fails the run if any of
those invariants is false.

## Run it

Clone the repository and run from its root:

```bash
export OPENAI_API_KEY="your-key"
cargo run -p everruns-bashkit-repo-agent
```

The default working copy is temporary. Pass a scratch directory after `--` to
keep it. Never pass a repository you care about: the agent may modify anything
inside the mounted workspace.

```bash
cargo run -p everruns-bashkit-repo-agent -- /tmp/release-run
```

Provider access and funded credits are required. Missing credentials,
unsuccessful turns, or failed host assertions exit nonzero.

## Build the agent

The editable prompt lives in `instructions.md`. Writing is an explicit policy
choice; the default workspace policy is read-only.

```rust
let agent = Agent::builder()
    .name("bashkit-repo-agent")
    .instructions(include_str!("../instructions.md"))
    .provider(provider)
    .model("gpt-5.6-terra")
    .workspace(workspace)
    .workspace_policy(WorkspacePolicy::read_write())
    .capability(BashkitShell::new())
    .build()?;
```

## Run and verify

The shared observer subscribes before sending, displays bounded shell events,
and waits for a successful turn. The host then reads the workspace itself;
confidence in the final answer cannot make a failed release pass.

```rust
let session = Engine::new().create(agent);
demo::run(&session, &release_request(&release_date)).await?;

let checks = verify(&root, &release_date);
if checks.iter().any(|check| !check.passed) {
    return Err("release verification failed".into());
}
```

Use `session.send_and_wait(request).await?` when a live tool timeline is not
needed.

## How the sandbox behaves

Bashkit interprets bash in-process against `/workspace`: there is no subprocess,
network, Git, or host filesystem outside the mount. Commands, loops, and script
size are bounded. The recording shows the model encountering an unsupported
`find -delete`, inspecting partial state, and recovering with a portable command.

## Validate it

```bash
cargo test -p everruns-bashkit-repo-agent
python3 examples/bashkit-repo-agent/render_demo.py --check
```

These checks are offline. They validate construction, fixture state, host-side
assertions, and recording pagination; they do not grade model quality.

## Recording workflow

[`demo.txt`](https://github.com/everruns/everruns/blob/main/examples/bashkit-repo-agent/demo.txt)
is a successful live transcript. `render_demo.py` automatically paginates it,
and VHS replays those pages without another provider call:

```bash
cd examples/bashkit-repo-agent
bash record.sh
```

The script preserves the previous successful transcript if the live run fails.
Review output before sharing when adapting this observer to private data.

## Adapt it safely

Replace the fixture, request, and `verify` assertions together. Keep the mount
narrow, prefer a disposable copy, grant write access only when necessary, and
verify consequential claims from host state after the turn.

## Boundaries

This is one sandboxed repository operation, not a general coding agent. It has
no network or Git credentials and cannot commit or push. Its Framework session
is in-memory.
