# Host Shell Agent

Fix a failing Rust test suite by compiling and running it, through the
`host_shell` capability: real processes on this machine, bounded by a kernel
policy. This is a real `gpt-5.6-terra` agent, not a scripted turn.

## What you learn

How to give an agent the machine it is running on without giving it the machine:
one execution capability, a kernel-enforced write and network boundary, and
independent host assertions as the success condition.

This is the example the sandboxed [Bashkit shell](../bashkit-repo-agent) cannot
be. Compiling a crate and running its test binary needs a real toolchain and
real child processes; an in-process interpreter over a virtual filesystem has
neither.

## Scenario and expected outcome

`src/resources/sample-crate/` is a small zero-dependency crate whose `chunk_count`
drops the partial final chunk, so two tests fail. The agent must run the suite,
read the failure, fix the source, and re-run until it passes.

Before the agent starts, the example probes the same containment provider the
capability uses and prints what it found: a write inside the workspace succeeds,
a write outside it is refused by the kernel, an outbound socket is refused by the
kernel. Not a description of the boundary, the boundary.

After the turn, Rust code re-runs `cargo test` from the host and fails the
process if the suite is still red, if an assertion was edited away, or if a test
was marked `#[ignore]`. A confident model answer cannot make a red suite pass.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. This folder is
self-contained **within the workspace** because its manifest references local
Framework crates.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export OPENAI_API_KEY="your-key"
cargo run -p everruns-host-shell-agent
```

Type the task interactively, or point it at a directory you keep:

```bash
cargo run -p everruns-host-shell-agent -- --interactive
cargo run -p everruns-host-shell-agent -- /tmp/chunker
```

With no directory, the fixture is materialized into a fresh temporary one and
discarded afterwards.

## Requirements

Kernel containment needs macOS, or Linux with Landlock ABI v3 fully enforced
(Linux 6.2 or a backport). On a kernel that cannot enforce it, the provider
fails closed and the example says so instead of running uncontained.

On Linux the policy is applied by a helper process. `everruns-host` ships one as
`everruns-sandbox-exec`, but cargo does not build a dependency's binaries, so
this example is its own helper instead: `main` routes a `__sandbox-exec`
re-exec into the containment worker and names that through
`SandboxLauncher::ReexecSelf`. That is what a single-binary embedder does, and
it needs nothing else on disk.

## What to read

- [`src/agent.rs`](src/agent.rs), the whole agent: one capability, one
  containment mode.
- [`src/fixture.rs`](src/fixture.rs), the independent verdict.
- [`src/resources/instructions.md`](src/resources/instructions.md), what the
  agent is told about the boundary it works inside.
