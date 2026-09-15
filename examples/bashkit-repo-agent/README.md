# Bashkit Repo Agent

Cut a release in a disposable repository through the sandboxed
[Bashkit](https://bashkit.sh) shell, then verify every claimed change directly
from the host. This is a real `gpt-5.6-terra` agent, not a scripted turn.

![Bashkit Repo Agent terminal demo](demo/demo.gif)

## What you learn

How to mount a narrow read-write workspace, give an agent one execution
capability, accept an interactive task, and treat independent host assertions
as the success condition.

## Scenario and expected outcome

The bundled `src/resources/sample-repo/` is a two-crate Cargo workspace with
three changelog fragments. The agent must cut release `0.2.0`: update both
package versions and the path-dependency pin, create today's changelog section,
preserve the `0.1.0` history, and remove the folded fragments.

After the turn, Rust code reopens the real working copy and fails the process if
any invariant is false. A confident model answer cannot make a broken release
pass.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. This folder is
self-contained **within the workspace** because its manifest references local
Framework crates.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export OPENAI_API_KEY="your-key"
cargo run -p everruns-bashkit-repo-agent
```

Type the task interactively:

```bash
cargo run -p everruns-bashkit-repo-agent -- --interactive
```

The default workspace is temporary and is removed on exit. Pass a scratch
directory to inspect the result afterwards:

```bash
cargo run -p everruns-bashkit-repo-agent -- /tmp/release-run
cargo run -p everruns-bashkit-repo-agent -- --interactive /tmp/release-run
```

The configured model is `gpt-5.6-terra`. Provider access and funded credits are
required; missing credentials, unsuccessful turns, and failed disk assertions
exit nonzero.

Never pass a repository you care about: the example materializes its fixture
into the target and the agent may change anything inside the mount.

## Build the agent

The definition lives in `src/agent.rs`; the prompt and disposable repository
live under `src/resources/`. Read-write access is explicit—the default workspace
policy is read-only.

```rust
pub fn build(api_key: String, workspace: &Path) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("bashkit-repo-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(OpenAI::new(api_key))
        .model(MODEL)
        .max_iterations(12)
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_write())
        .capability(BashkitShell::new())
        .build()
}
```

## Run and verify

`main.rs` keeps orchestration readable. The shared demo observer displays a
bounded shell timeline and waits for a successful turn. The host then verifies
the mounted files itself.

```rust
let agent = agent::build(api_key, &workspace)?;
let engine = Engine::new();
let session = engine.create(agent);

demo::run(&session, &request).await?;
verify_release(&workspace, &release_date)?;
```

Use `session.send_and_wait(&request).await?` when a live tool timeline is not
needed.

## How Bashkit behaves

Bashkit interprets shell scripts in-process against `/workspace`. The model gets
no host filesystem outside that mount, network, Git credentials, or subprocess
execution. Commands, loops, output, and script size are bounded. Repository text
is treated as untrusted data rather than agent instructions.

## Validate it

```bash
cargo test -p everruns-bashkit-repo-agent
bash examples/bashkit-repo-agent/demo/record.sh --check
```

Tests validate argument handling, the fixture's initial state, and host-side
assertions without provider credentials. CI does not grade nondeterministic
model output, so a live run remains the behavioral proof.

## Demo and recording

The screencast types the release task into the same interactive binary shown
above, displays real `bashkit_shell` calls, and ends with host-side checks over
the mutated files. It does not replay a prepared transcript. Read
`demo/transcript.txt` when a static log is easier to inspect.

With VHS, ffmpeg, a VHS-compatible browser, and funded OpenAI credentials:

```bash
bash examples/bashkit-repo-agent/demo/record.sh
```

The script uses `OPENAI_API_KEY` when exported, otherwise Doppler project
`everruns-dev`, config `dev`. It updates the checked-in GIF and transcript only
after the model turn and host verification succeed.

## Adapt it safely

Replace the fixture, request acceptance criteria, and verifier together. Keep
the mount disposable and narrow, start read-only unless mutation is required,
and verify consequential claims outside the model/tool boundary.

## Boundaries

This demonstrates one sandboxed repository mutation, not a general coding
agent. It cannot fetch dependencies, run native programs, commit, or push. The
Framework session is in-memory.

## Source map

`src/main.rs`: input, session, and verification flow; `src/agent.rs`: agent
definition; `src/fixture.rs`: fixture materialization and assertions;
`src/resources/`: prompt and sample repository; `demo/`: live VHS recording and
transcript. `examples/demo-support::shell` provides terminal presentation.

## See also

- [Bashkit Shell capability](https://docs.everruns.com/capabilities/bashkit-shell/)
- [`coding-cli`](../coding-cli) — a fuller terminal coding agent
