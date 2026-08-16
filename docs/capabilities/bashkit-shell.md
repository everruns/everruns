---
title: Bashkit Shell
description: Sandboxed Bash command execution in an isolated environment. Agents can run shell commands safely with process isolation, resource limits, streaming output, and workspace-only filesystem access.
---

| | |
|---|---|
| **ID** | `bashkit_shell` (legacy alias: `virtual_bash`) |
| **Category** | Execution |
| **Risk** | High, assignment requires an org **Admin** |
| **Features** | `file_system` (enables the Workspace tab) |
| **Dependencies** | [`session_file_system`](/capabilities/file-system/) |

Execute bash commands in a sandboxed environment with no access to the host
system. The session filesystem is mounted at `/workspace`, so commands read and
write the same files as the [File System](/capabilities/file-system/) tools.

## Powered by Bashkit

This capability runs on [**bashkit**](https://bashkit.sh), an embeddable bash
interpreter that executes shell scripts in-process inside a WASM-like sandbox,
with no real shell, no subprocess spawning, and no host access. Learn more at
[bashkit.sh](https://bashkit.sh) or browse the source on
[GitHub](https://github.com/everruns/bashkit).

Because the interpreter is sandboxed by construction, bash here is **not** a
shell-out to the host: there is no `/bin/bash` process, no direct network stack,
and no filesystem beyond the session workspace. Outbound HTTP for `curl`/`wget`
is off by default and can be enabled per agent (see **Outbound HTTP** below).

## Tools

### `bash`

Execute a shell command (or a multi-line script).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `commands` | string | yes | Shell command(s) to execute |
| `working_dir` | string | no | Working directory (default: `/workspace`) |
| `timeout_ms` | integer | no | Timeout in milliseconds (default: `30000`, max: `60000`) |
| `output` | string | no | Output verbosity (`auto`, `normal`, …; default: `auto`) |

Returns `stdout`, `stderr`, `exit_code`, and a `success` flag. Output streams
live to the UI and CLI via `tool.output.delta` events while the command runs.
On timeout, any partial output captured so far is returned alongside the error.

This tool also supports background execution, long scripts can run detached and
report progress without blocking the agent loop.

## Filesystem

The interpreter exposes a single mount:

- **`/workspace`** maps to the session file store. Reads and writes are live,
  files created by bash are immediately visible to the File System tools and
  vice versa.
- Paths outside `/workspace` (for example `/etc`, `/home/agent`, `/tmp`) do not
  exist and cannot be written.
- Symlinks are unsupported; `chmod` is a no-op (the session filesystem does not
  track Unix permissions, and files are executable by default).

Default environment: `HOME=/home/agent`, `SHELL=/bin/bash`,
`PATH=/usr/local/bin:/usr/bin:/bin`, `WORKSPACE=/workspace`, user and host
`everruns`.

## Resource limits

Every invocation runs under fixed limits to prevent runaway scripts:

| Limit | Value |
|---|---|
| Max commands per run | 1,000 |
| Max loop iterations | 10,000 |
| Max function depth | 100 |
| Max script size | 1 MB |
| Max memory | 10 MB |
| Parser timeout | 5 s |
| Wall-clock timeout | `timeout_ms` (default 30 s, max 60 s) |

## Outbound HTTP (optional)

Set the capability config `{"enable_http": true}` to let scripts use `curl` and
`wget`. Every request, including each redirect hop, is routed through the
platform egress boundary, where the agent/session network access list and the
deployment-wide system allowlist are enforced. Policy denials surface as curl's
native `access denied` failure (exit code 7). Without the flag, the interpreter
has no network path at all.

## Security

- **Sandboxed**: no direct network access (outbound HTTP is opt-in and
  egress-routed), no host filesystem, no subprocess spawning.
- **High risk**: because it exposes arbitrary scripted code execution, assigning
  `bashkit_shell` to an agent requires an org **Admin**. Existing agents that
  already had it keep working; the gate applies to new assignments only.
- Built-in observability hooks emit structured `tracing` events per builtin and
  on interpreter errors (tagged with the session ID) without logging argument
  values or command output.

## Notes

- Commands operate on the same `/workspace` as
  [File System](/capabilities/file-system/) tools.
- Built-in commands support `<command> --help`, and many also support
  `<command> --version`.
- Common builtins: `cd`, `ls`, `cat`, `echo`, `grep`, `head`, `tail`, `sed`,
  `find`, plus shell features like pipes, redirections, and command
  substitution. `grep` is backed by the session's indexed search.

## See Also

- [Bashkit project](https://bashkit.sh), the interpreter powering this capability
- [File System](/capabilities/file-system/), file operations on the same workspace
- [Sub Agents](/capabilities/sub-agents/), background and parallel execution
- [Capabilities Overview](/capabilities/)
