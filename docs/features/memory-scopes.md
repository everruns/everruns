---
title: Agent and User Memory
description: Understand organization, agent, and user memory scopes, their automatic mount paths, and the privacy and access rules for each.
---

Memory keeps files available across sessions. Everruns provides three ownership scopes so shared organization knowledge, an agent's learned context, and a user's preferences do not have to share the same access boundary.

## The three scopes

| Scope | Lifetime and owner | How it enters a session | Access |
|---|---|---|---|
| Organization | A named store managed within one organization | Select it in the `memory` capability and choose a path under `/workspace` | Read-only by default; read-write must be selected explicitly |
| Agent | One server-managed store per agent | Automatically mounted for sessions hosted by that agent at `/memory/agent` | Read-write in the hosted session |
| User | One server-managed store per user | Automatically mounted in the owning user's private, default session workspace at `/memory/user` | Read-write for the owner and the agent working in that session |

Organization memories are appropriate for shared references such as runbooks or product documentation. Agent memory follows one host agent across its sessions. User memory follows one user so preferences and personal context can persist across their sessions.

## Privacy defaults

**User memory is private by default.** Only the session owner may access `/memory/user` through user-facing session file APIs. Other users cannot read or mutate the subtree, and file searches redact matches from it. Internal runtime access lets the agent working for the owner read and update that memory.

User memory is not mounted into caller-attached shared workspaces. Everruns waits to expose it there until mounts can be participant-local instead of visible to the entire workspace.

Agent memory is separate from user memory. It is shared by sessions hosted by the same agent, so do not use `/memory/agent` for user-private data. A guest agent's private memory is not merged into a shared session's workspace-wide `/memory/agent` path.

## How automatic mounts work

When a session starts, Everruns lazily creates any missing scoped stores and mounts their current files:

- `/memory/agent` contains the host agent's persistent files.
- `/memory/user` contains the session owner's persistent files when the session uses the owner's private default workspace.

Both automatic mounts are read-write. The regular file tools and Workspace UI traverse them alongside session files, and writes persist to the scoped memory for later sessions.

The `/memory/*` namespace is reserved. You cannot use it for caller-supplied initial files or for mounts configured through the public `memory` capability.

## Organization memory access

Organization memories are created and managed through the Memory UI and `/v1/memories` API. Add the `memory` capability to select an active organization memory, its mount path, and its access mode.

- Omitted access mode defaults to `readonly`.
- A `readwrite` mount writes through to the durable organization memory.
- Source-backed GitHub or Git memories are always read-only.
- Agent- and user-scoped memories are server-managed and cannot be selected through `memory.mounts`.

See [Memory model](/advanced/memory-model/) for the relationship between durable Memory and a session Workspace, or the [API reference](/api/) for current organization-memory schemas.

## See also

- [Memory model](/advanced/memory-model/), Workspace and durable Memory architecture.
- [Session participants](/features/session-participants/), host and guest agent behavior in shared sessions.
- [File System capability](/capabilities/file-system/), tools that access session and mounted files.
