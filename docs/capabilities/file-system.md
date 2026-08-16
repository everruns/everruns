---
title: File System
description: Read, write, search, and manage files in an isolated per-session workspace. Agents get sandboxed file access with glob, grep, and directory operations.
---

| | |
|---|---|
| **ID** | `session_file_system` |
| **Category** | File Operations |
| **Features** | `file_system` (enables Workspace tab) |
| **Dependencies** | None |

Provides tools to access and manipulate files in the session workspace. Each session has an isolated filesystem rooted at `/workspace`. Files persist for the session duration. `read_file` and `write_file` return a `content_hash` (`sha256:...`) so agents can make freshness-checked `edit_file` calls.

## Tools

### `read_file`

Read the contents of a file. Successful responses include `content_hash`.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Absolute path (e.g., `/workspace/src/main.py`) |

### `write_file`

Create or overwrite a file. Parent directories are created automatically. Successful responses include `content_hash`.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Absolute path |
| `content` | string | yes | File content |

### `edit_file`

Apply one or more exact text replacements to an existing text file. This tool is text-only, requires the current `content_hash` from `read_file` or `write_file`, and uses compare-and-set semantics so concurrent writes fail cleanly instead of clobbering newer content.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | Absolute path to an existing text file |
| `expected_hash` | string | yes | Current `content_hash` (`sha256:...`) |
| `edits` | array | yes | One or more `{ old_text, new_text }` replacements matched against the original file. Use a single-element array for one replacement. |

Legacy top-level `old_text`/`new_text` are still accepted for backward compatibility, they are folded into `edits[]`, but new callers should always use `edits[]`.

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

## `edit_file` request example

```json
{
  "path": "/workspace/app.py",
  "expected_hash": "sha256:1c4d...",
  "edits": [
    {
      "old_text": "return 'Hello, World!'",
      "new_text": "return 'Hello from Everruns!'"
    }
  ]
}
```

## Notes

- All paths must be under `/workspace`
- Files are session-scoped, no cross-session access
- Parent directories are auto-created on write
- `edit_file` only works on text files and rejects binary/base64 content
- `edit_file` applies all replacements against the original file content and rejects ambiguous or overlapping matches
- `edit_file` preserves the file's existing BOM and newline style (`LF`, `CRLF`, or `CR`)
- `edit_file` returns a unified diff capped to a bounded size; oversized diffs are truncated and marked as such
- Shared filesystem with [Bashkit Shell](/capabilities/bashkit-shell/) (same `/workspace`)

## See Also

- [Bashkit Shell](/capabilities/bashkit-shell/), execute commands against these files
- [Storage](/capabilities/session-storage/), key/value and secret storage
- [Capabilities Overview](/capabilities/)
