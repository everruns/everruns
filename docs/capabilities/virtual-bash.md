---
title: Virtual Bash
description: Sandboxed bash command execution in an isolated environment
---

| | |
|---|---|
| **ID** | `virtual_bash` |
| **Category** | Execution |
| **Risk** | High (admin required) |
| **Features** | `file_system` (unlocks Workspace tab) |
| **Dependencies** | [`session_file_system`](/capabilities/file-system/) |

Execute bash commands in a sandboxed environment with no access to the host system. Powered by [bashkit](https://github.com/everruns/bashkit), a WASM-like execution sandbox. The session filesystem is mounted at `/workspace`.

## Tools

### `bash`

Execute a shell command.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `command` | string | yes | Shell command to execute |
| `working_dir` | string | no | Working directory (default: `/workspace`) |
| `timeout` | integer | no | Timeout in seconds |

Returns stdout, stderr, and exit code.

## Use Cases

- **Code execution** — run scripts, compile code, execute tests
- **Data processing** — pipe commands, transform files with standard Unix tools
- **Environment setup** — install packages, configure environments
- **Build automation** — run build tools, linters, formatters

## Example

Agent runs a Python script and inspects the output:

```
User: Run the test suite

Agent:
  → bash("cd /workspace && python -m pytest tests/ -v")
  ← stdout: "tests/test_api.py::test_health PASSED\ntests/test_api.py::test_create PASSED\n2 passed in 0.34s"
  ← exit_code: 0
```

## Notes

- **Sandboxed** — no network access, no host filesystem access
- Commands operate on the same `/workspace` as [File System](/capabilities/file-system/) tools
- Files written by bash are immediately visible to file system tools and vice versa
- Built-in shell commands: `cd`, `ls`, `cat`, `echo`, `grep`, `head`, `tail`, etc.
- Resource limits prevent infinite loops

## See Also

- [File System](/capabilities/file-system/) — file operations on the same workspace
- [Capabilities Overview](/capabilities/)
