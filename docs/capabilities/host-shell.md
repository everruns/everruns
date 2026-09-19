---
title: Host Shell
description: Run bash commands on the machine hosting the agent, bounded by a kernel policy (Landlock and seccomp on Linux, Seatbelt on macOS).
---

| | |
|---|---|
| **ID** | `host_shell` |
| **Category** | Execution |
| **Risk** | High |
| **Features** | `file_system` (enables the Workspace tab) |
| **Dependencies** | [`session_file_system`](/capabilities/file-system/), backed by a real directory |

Run bash commands as real child processes on the machine the agent is running
on. Unlike [Bashkit Shell](/capabilities/bashkit-shell/), the toolchain is real:
compilers, package managers and test runners work. A kernel policy bounds what
those processes may write and whether they may reach the network.

## When to use this instead of Bashkit

Both capabilities contribute a tool named `bash` over the session workspace, so
enable one or the other, not both.

| | Bashkit Shell | Host Shell |
|---|---|---|
| Where commands run | In-process interpreter | This machine, as child processes |
| Native binaries | No | Yes |
| Workspace | Session filesystem (virtual or real) | Must be a real directory |
| Boundary | The interpreter, by construction | Landlock + seccomp, or Seatbelt |
| Network | Off, or egress-routed HTTP | Denied unless containment is removed |

Given a virtual session filesystem, `host_shell` refuses and says so rather than
inventing a host path.

## Tools

### `bash`

Execute a shell command or a multi-line script.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `command` | string | yes | Shell command(s) to execute |
| `working_dir` | string | no | Directory to run in (default: the workspace root) |
| `sandbox_permissions` | string | no | `use_default` or `require_escalated` |
| `justification` | string | no | User-facing reason for `require_escalated` |
| `output` | string | no | Output verbosity (`auto`, `normal`, …; default: `auto`) |

`commands` is accepted as an alias for `command`, so an agent written against
Bashkit Shell keeps working when the backend is swapped.

Returns `stdout`, `stderr`, `exit_code`, `success`, and the `containment` that
was in force. A failure that the containment would explain is flagged with
`containment_denial: "likely"`. Output streams live to the UI and CLI while the
command runs, and long scripts can run detached.

Every call is a fresh non-interactive `bash -lc` (PowerShell on Windows) rooted
at the workspace, so no working directory, variable or export survives between
calls.

## Containment

Configured per agent, never by the model.

| Mode | Reads | Writes | Network |
|---|---|---|---|
| `read-only` | the host | private temp only | denied |
| `workspace-write` (default) | the host | workspace, `/tmp`, private temp, configured roots | denied |
| `danger-full-access` | everything | everything | allowed |

Host **reads** are allowed in every contained mode, for toolchain
compatibility. The policy stops writes and network exfiltration; it does not
stop a command reading unrelated files on the machine. That is the threat model,
stated rather than implied.

The environment a command inherits is an allowlist: `PATH`, locale, and
toolchain variables survive; `HOME` and `TMPDIR` are replaced with a private
per-process directory, and everything else, including every API key and agent
socket path, is dropped.

Two platform differences are deliberate:

- `.git` below the workspace is read-only on macOS. Landlock path rules are
  additive and cannot subtract it, so Linux permits Git metadata writes inside
  the workspace.
- Windows has no containment implementation. Every mode there runs uncontained,
  and the capability says so.

On macOS and Linux, containment fails closed: if the OS primitive is
unavailable, the command returns a setup error and is not retried on the host.

## Configuration

```json
{
  "containment": "workspace-write",
  "approval": "never",
  "writable_roots": ["/var/cache/agent"],
  "foreground_timeout_secs": 120,
  "background_timeout_secs": 86400,
  "max_output_bytes": 1048576
}
```

An unknown `containment` or `approval` name is rejected rather than defaulted, so
a misspelled boundary cannot resolve to a wider one.

`writable_roots` adds directories a build needs to write beyond the workspace, a
package cache for example. It is ignored at `read-only`.

## Approvals

| Policy | When a person is asked |
|---|---|
| `never` (default) | never; a request to escalate is refused |
| `on-failure` | when a command fails in a way the containment would explain |
| `on-request` | when the model sets `sandbox_permissions: require_escalated` with a justification |
| `untrusted` | for anything outside a small read-only command set |

Every policy except `never` needs the host to supply an approval gate. Without
one, a policy that would ask refuses instead: an unattended worker has nobody to
ask, and a refusal is more honest than a silent escalation.

Regardless of policy, a command that visibly signals the agent's own process is
refused before it is spawned.

## Deployment

This is an embedder capability, not a hosted-product one. It ships in
`everruns-host` behind the `host-shell` feature (also reachable as `host-shell`
on the `everruns` facade) and is deliberately absent from the hosted catalog:
handing agents arbitrary host processes is something a CLI host, a CI runner, or
an operator's own box opts into, not something a shared multi-tenant worker
should offer.

On Linux the kernel policy is applied by a helper process, selected with the
`launcher` config key:

| `launcher` | Meaning |
|---|---|
| `"discover"` (default) | find `everruns-sandbox-exec` beside the binary, then on `PATH` |
| `{"helper": "<path>"}` | run that binary |
| `{"reexec_self": ["<arg>"]}` | re-exec this binary with those leading arguments |

`everruns-host` ships `everruns-sandbox-exec` under the same feature, but cargo
does not build a dependency's binaries, so a single-binary host will not find
one beside it. Such a host routes the arguments into
`everruns_host::containment::worker::run_from_args` from its own `main` and
selects `reexec_self`. See `examples/host-shell-agent`.

## See Also

- [Bashkit Shell](/capabilities/bashkit-shell/), the sandboxed interpreter
- [File System](/capabilities/file-system/), file operations on the same workspace
- [Daytona](/capabilities/daytona/) and [E2B](/capabilities/e2b/), real binaries on someone else's machine
- [Capabilities Overview](/capabilities/)
