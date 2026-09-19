# everruns-integrations-host-shell

> The `bash` tool over real host processes, bounded by a kernel policy.

Everruns already had two shells that could not be the third. `bashkit_shell`
interprets a script against the session filesystem with no host underneath it,
which is exactly right until an agent needs `cargo`, `git`, or anything else
that is a binary. The remote sandboxes (Daytona, E2B, Deno) run real binaries,
on a machine somewhere else. This capability is the case in between: real
processes, on the machine the agent is already running on.

The boundary lives in [`everruns-containment`](../../crates/containment). This
crate is the agent-facing half.

Part of the [Everruns](https://everruns.com) ecosystem. Framework applications
enable it with the `host-shell` feature; advanced hosts can register the
capability directly.

## What It Provides

- A `bash` tool running real child processes rooted at the session workspace
- Per-agent containment, approval policy, writable roots, timeouts, output budget
- Background execution with live `stdout`/`stderr` streaming
- An approval seam a host fills to put a person in front of a command
- A refusal, not a guess, when the workspace is not a real directory

## Capability

| | |
|---|---|
| **ID** | `host_shell` |
| **Category** | Execution |
| **Risk** | High |
| **Dependencies** | `session_file_system`, backed by a real directory |

## Configuration

```rust
use everruns_containment::ContainmentMode;
use everruns_integrations_host_shell::{ApprovalPolicy, HostShell};

let shell = HostShell::new()
    .containment(ContainmentMode::WorkspaceWrite)
    .approval(ApprovalPolicy::OnRequest)
    .writable_root("/var/cache/agent");
```

| Key | Default | Meaning |
|---|---|---|
| `containment` | `workspace-write` | `read-only`, `workspace-write`, or `danger-full-access` |
| `approval` | `never` | `never`, `on-failure`, `on-request`, `untrusted` |
| `writable_roots` | `[]` | Directories writable beyond the workspace and `/tmp` |
| `foreground_timeout_secs` | `120` | Wall clock for one call |
| `background_timeout_secs` | `86400` | Wall clock for a detached call |
| `max_output_bytes` | `1048576` | Captured per stream before the command is killed |

An unknown containment or approval name is rejected rather than defaulted: a
misspelled boundary must not resolve to a wider one.

## Approvals

Containment decides what a command may touch; approval decides whether it runs
at all. Only one of those can be answered without a person, so a host installs a
gate through the tool-context extension seam:

```rust
# use std::sync::Arc;
# use everruns_integrations_host_shell::{HostShellApproval, ShellApprovalGate};
# fn install(gate: Arc<dyn ShellApprovalGate>, extensions: &mut everruns_core::tool_context::ToolContextExtensions) {
extensions.insert(Arc::new(HostShellApproval::new(gate)));
# }
```

With no gate installed, every policy that would ask refuses instead. An
unattended worker has nobody to ask, and a refusal is the honest answer.

## Choosing between this and `bashkit_shell`

Both contribute a tool named `bash` over the session workspace, so an agent
enables one or the other. Pick `bashkit_shell` when the workspace is virtual and
the work is shell-shaped operations over files. Pick `host_shell` when the agent
needs the machine: a toolchain, a package manager, a test suite.

The tool accepts both `command` (this crate and Yolop) and `commands`
(`bashkit_shell`) as the script argument, so swapping the execution backend
under an agent does not invalidate its prompt.

`host_shell` requires a session filesystem backed by a real directory. Given a
virtual one it refuses and says so, rather than inventing a path.

## Example

[`examples/host-shell-agent`](../../examples/host-shell-agent) hands an agent a
Rust crate with a red test suite and nothing but this capability, then re-runs
the suite from the host to check the answer.

## Documentation

- [Host Shell capability](https://docs.everruns.com/capabilities/host-shell/)
- [API reference](https://docs.rs/everruns-integrations-host-shell)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
