---
title: Memory
description: Mount named organization Memories into the session workspace, read-only by default or read-write as shared working memory across sessions.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `memory` |
| **Category** | Memory |
| **Tools** | None |
| **Features** | `file_system` |
| **Dependencies** | [File System](/capabilities/file-system/) |
| **Risk** | Medium |

The `memory` capability mounts organization Memories into a session's
`/workspace`. A Memory is a named, durable file store owned by the
organization. Once mounted, the agent reads and writes it with the ordinary
[file tools](/capabilities/file-system/), so the capability adds no tools of its
own.

Use it for reference material that many sessions share, such as runbooks or
product documentation, or for working files that should outlive one session.

This capability selects **organization** Memories only. Agent memory
(`/memory/agent`) and user memory (`/memory/user`) are mounted by the server
without this capability. [Agent and User Memory](/features/memory-scopes/)
explains the three scopes and their privacy rules.

## Configuration

```json
{
  "ref": "memory",
  "config": {
    "mounts": [
      {
        "memory": "mem_0193f0c2a1b27c4e8f5d6a7b8c9d0e1f",
        "path": "/workspace/runbooks",
        "mode": "readonly"
      }
    ]
  }
}
```

| Field | Required | Description |
|---|---|---|
| `mounts[].memory` | Yes | Memory ID, `mem_` followed by 32 lowercase hex characters |
| `mounts[].path` | Yes | Absolute path under `/workspace` where the Memory appears |
| `mounts[].mode` | No | `readonly` (default) or `readwrite` |

An empty config, or no `mounts`, is valid and mounts nothing.

Validation rejects:

- a path outside `/workspace`
- two mounts with the same path
- overlapping paths, where one mount path is a parent of another
  (`/workspace/data` and `/workspace/data/inner`)

The `/memory/*` namespace is reserved for the server-managed scopes and cannot
be used as a mount path here.

## Access modes

- `readonly` mounts reject writes with an error from the file tools.
- `readwrite` mounts write through to the durable Memory, so later sessions
  see the change.
- Memories synced from GitHub or Git are always read-only, whatever mode the
  mount asks for.

The mount set is captured when the session is created. Archiving or renaming a
Memory later does not change what a running session mounted.

## Managing Memories

Create Memories and edit their files in the **Memory** page of the UI or
through the `/v1/memories` API. The [API reference](/api/) has the request
shapes.

## See also

- [Agent and User Memory](/features/memory-scopes/), the three memory scopes
- [Knowledge Base](/capabilities/knowledge-base/), curated entries the agent searches instead of reading files
- [Knowledge Index](/capabilities/knowledge-index/), semantic search over a synced source
- [File System](/capabilities/file-system/), the tools that read and write mounted files
