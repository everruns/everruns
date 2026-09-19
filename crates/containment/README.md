# everruns-containment

> Kernel containment for the commands an agent runs on this machine.

Seatbelt on macOS, Landlock plus seccomp on Linux. `everruns-host` answers *where* a command runs. This package answers *what that
command may touch*, and is the implementation behind
`ContainmentLevel::Native`. It was extracted from
[Yolop](https://github.com/everruns/yolop), which has shipped and tested it on
both platforms since before Everruns had a host target at all.

## Why its own package

The layer is consumed by two release trains: Everruns' runtime and Yolop's
terminal agent. A package whose only dependencies are the OS primitives can be
pinned by both without either dragging the other along, the way Yolop already
pins Tuika. Everything above it, the capability, the tool, the agent config,
lives in `everruns-integrations-host-shell`.

Part of the [Everruns](https://everruns.com) ecosystem. Framework applications
reach it through the `host-shell` feature; a host that wants the boundary
without the tool depends on this package directly.

## What It Provides

- `SandboxProvider`, contained `bash`/PowerShell processes per containment mode
- Seatbelt profiles on macOS; Landlock plus a seccomp socket filter on Linux
- A sanitized child environment: toolchain variables in, credentials out
- `policy`, parsed checks for self-signalling and destructive shell actions
- `everruns-sandbox-exec`, the Linux helper that restricts itself and execs bash

## Two invariants

1. **Model input never selects a host executable or widens a mount.** A caller
   configures `SandboxOptions` once; a tool passes only a script.
2. **It fails closed.** When the OS primitive a mode needs is unavailable,
   `SandboxProvider::command` returns an error instead of degrading to an
   uncontained process. Windows is the documented exception: it has no
   implementation, so `danger_warning` fires at every mode there.

## Usage

```rust
use everruns_containment::{ContainmentMode, SandboxOptions, configure_stdio, provider};

# async fn run() -> anyhow::Result<()> {
let sandbox = provider(SandboxOptions::new(ContainmentMode::WorkspaceWrite));
let mut command = sandbox.command(std::path::Path::new("/srv/workspace"), "cargo test")?;
configure_stdio(&mut command);
let output = command.output().await?;
# let _ = output;
# Ok(())
# }
```

## The Linux helper

Landlock and seccomp restrict the calling process, and must be installed before
the shell is exec'd. Doing that from `pre_exec` would allocate and take locks
after forking a multi-threaded runtime, so a helper process does it instead: it
restricts itself, then execs bash in place.

This package ships that helper as `everruns-sandbox-exec`, and
`SandboxLauncher::Discover` finds it next to the current executable or on
`PATH`. A single-binary embedder can route into `worker::run_from_args` from its
own `main` and select `SandboxLauncher::ReexecSelf` instead.

## What it does not do

- macOS and Linux both allow host **reads**. The policy stops writes and network
  exfiltration; it does not stop a command reading unrelated files. That is the
  stated threat model, not an implied guarantee.
- `/tmp` is writable at `workspace-write` for development-tool compatibility, so
  a contained command can leave files where other host processes see them.
- `.git` below the workspace is read-only on macOS. Landlock path rules are
  additive and cannot subtract it, so Linux permits Git metadata writes inside
  the workspace. The platforms differ here by construction.
- Windows has no containment implementation yet.

## Documentation

- [Host Shell capability](https://docs.everruns.com/capabilities/host-shell/)
- [API reference](https://docs.rs/everruns-containment)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
