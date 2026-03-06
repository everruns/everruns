---
title: File System
description: Read, write, search, and manage files in an isolated per-session workspace
---

| | |
|---|---|
| **ID** | `session_file_system` |
| **Category** | File Operations |
| **Features** | `file_system` (unlocks Workspace tab) |
| **Dependencies** | None |

Provides tools to access and manipulate files in the session workspace. Each session has an isolated filesystem rooted at `/workspace`. Files persist for the session duration.

## Tools

### `read_file`

Read the contents of a file.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Absolute path (e.g., `/workspace/src/main.py`) |

### `write_file`

Create or overwrite a file. Parent directories are created automatically.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Absolute path |
| `content` | string | yes | File content |

### `list_directory`

List files and directories at a given path.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Directory path |

### `grep_files`

Search file contents with regex patterns.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `pattern` | string | yes | Regex pattern |
| `path` | string | no | Directory to search (default: `/workspace`) |

### `delete_file`

Delete a file or directory.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Path to delete |

### `stat_file`

Get file metadata (size, type, timestamps).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Path to stat |

## Use Cases

- **Code generation** — agent writes source files, configs, and scripts to the workspace
- **Document processing** — read uploaded files, transform content, write results
- **Project scaffolding** — create directory structures with boilerplate files
- **Log analysis** — grep through log files for patterns and errors

## Example

Agent creates a Python project:

```
User: Create a Flask hello world app

Agent:
  → write_file("/workspace/app.py", "from flask import Flask\napp = Flask(__name__)\n\n@app.route('/')\ndef hello():\n    return 'Hello, World!'\n")
  → write_file("/workspace/requirements.txt", "flask>=3.0\n")
```

## Notes

- All paths must be under `/workspace`
- Files are session-scoped — no cross-session access
- Parent directories are auto-created on write
- Shared filesystem with [Virtual Bash](/capabilities/virtual-bash/) (same `/workspace`)

## See Also

- [Virtual Bash](/capabilities/virtual-bash/) — execute commands against these files
- [Storage](/capabilities/session-storage/) — key/value and secret storage
- [Capabilities Overview](/capabilities/)
