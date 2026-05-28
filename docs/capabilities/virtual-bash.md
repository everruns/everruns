---
title: Virtual Bash
description: Sandboxed Bash command execution in an isolated environment. Agents can run shell commands safely with process isolation, timeouts, and output capture.
---

| | |
|---|---|
| **ID** | `virtual_bash` |
| **Category** | Execution |
| **Features** | `file_system` (unlocks Workspace tab) |
| **Dependencies** | [`session_file_system`](/capabilities/file-system/) |

Execute bash commands in a sandboxed environment with no access to the host system. Powered by [bashkit](https://github.com/everruns/bashkit), a WASM-like execution sandbox. The session filesystem is mounted at `/workspace`.

## Tools

### `bash`

Execute a shell command.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `command` | string | yes | Shell command to execute |
| `working_dir` | string | no | Working directory. When omitted, the session's last directory is resumed; when set, it overrides it for this call |
| `timeout` | integer | no | Timeout in seconds |

Returns stdout, stderr, and exit code.

## Notes

- **Sandboxed** — no network access, no host filesystem access
- **Stateful** — shell state (working directory, exported/shell variables, functions, aliases) persists across calls within a session, so bash behaves like a REPL
- Commands operate on the same `/workspace` as [File System](/capabilities/file-system/) tools
- Files written by bash are immediately visible to file system tools and vice versa
- Built-in shell commands: `cd`, `ls`, `cat`, `echo`, `grep`, `head`, `tail`, etc.
- Resource limits prevent infinite loops

## See Also

- [File System](/capabilities/file-system/) — file operations on the same workspace
- [Capabilities Overview](/capabilities/)
