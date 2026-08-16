---
title: Memory model
description: How Workspace and Memory relate, the two-tier model for what an agent can read and write.
---

Everruns separates **where the agent works** from **what the agent remembers across runs**. That split is the whole memory model:

- **Workspace**: the active working area for a session. Per-run, singleton, ephemeral by default. Mounted at `/workspace`.
- **Memory**: org-scoped, named stores. Durable, governed, mountable into Workspaces. RO by default.

These two tiers are intentional. Workspace is where an agent does its current task; Memory is where the org persists state it wants reused across tasks.

For the shipped organization, agent, and user ownership tiers, including the private `/memory/user` and automatic `/memory/agent` mounts, see [Agent and user memory](/features/memory-scopes/).

## The two-axis grid

Scope × surface, with the same surfaces appearing on both sides:

```text
                  SESSION                   ORG
                  (Workspace — per run)     (Memory — durable, shared)
          ┌────────────────────────┬────────────────────────┐
Files     │ workdir, scratch       │ shared docs, code      │
          ├────────────────────────┼────────────────────────┤
Tables    │ session SQL db         │ shared datasets (TBD)  │
          ├────────────────────────┼────────────────────────┤
KV        │ run state, notes       │ facts, prefs (TBD)     │
          ├────────────────────────┼────────────────────────┤
Secrets   │ per-run creds          │ org credentials (TBD)  │
          └────────────────────────┴────────────────────────┘
```

Today only the **Files** surface exists on both sides; Tables exist on the Workspace side (session SQL DB). Tabular, KV, secrets, and structured surfaces on Memory are durable design intent, see `knowledge/runtime-resources/memory.md`.

## Org → Session: Mount

A Memory does not enter a Workspace automatically. The `memory` capability declares which Memories are mounted, where, and with what access mode.

```text
                   ORG MEMORIES (named, many)
       ┌──────────────┬──────────────┬──────────────┐
       │ mem:crm      │ mem:legal-kb │ mem:pricing  │  ...
       └──────┬───────┴──────┬───────┴──────┬───────┘
              │              │              │       per-mount RO | RW
              ▼              ▼              ▼
       ┌─────────────────────────────────────────┐
       │            SESSION WORKSPACE            │
       │                                         │
       │   /workspace          ← native files    │
       │   /workspace/mnt/...  ← Memory mounts   │
       └─────────────────────────────────────────┘
```

A Memory can be mounted into any number of sessions concurrently. The mount is snapshotted at session creation, so later archival or rename of the Memory does not destabilize a running session.

## Session-eye view

What the running agent actually sees:

```text
                    SESSION WORKSPACE (one run)
   ┌────────────────────────────────────────────────────────┐
   │                                                        │
   │   NATIVE (lives and dies with the session)             │
   │   ┌──────────┬──────────┐                              │
   │   │ files    │ tables   │                              │
   │   │ /workspace  session SQL db                         │
   │   └──────────┴──────────┘                              │
   │                                                        │
   │   MOUNTED (projected from org Memories)                │
   │   ┌────────────────────────────────────────────────┐   │
   │   │ /workspace/mnt/docs    ← mem:design       [ro] │   │
   │   │ /workspace/mnt/orders  ← mem:crm          [rw] │   │
   │   │ /workspace/mnt/legal   ← mem:legal-kb     [ro] │   │
   │   └────────────────────────────────────────────────┘   │
   └────────────────────────────────────────────────────────┘
```

The same tools (`read_file`, `list_directory`, `grep_files`, `bashkit_shell`) traverse native and mounted paths uniformly. Writes to read-only mounts return clear errors; writes to read-write mounts write through to the underlying Memory and are audited.

## Why two tiers, not one

A single "everything is Memory" abstraction was considered and rejected for two reasons:

1. **Lifecycle and governance differ.** Workspace files are intermediate, agents probe, edit, discard. Memory is durable shared state with audit, lifecycle (active/archived/deleted), and trust boundaries. Forcing one set of policies on both was wrong for both.
2. **Naming clarity.** "Memory" anthropomorphizes what the agent recalls across tasks. "Workspace" describes the desk it's working on. Mixing the two names ("session memory" vs "org memory") forced every sentence to carry a qualifier.

External validation: Anthropic's Claude Managed Agents settled on essentially the same split, workspace files (per-run) plus Memory Stores (`/mnt/memory/`, durable, named, RO/RW mountable). The terminology in this doc mirrors that convention deliberately.

## Invariants

- **Scope.** Sessions cannot see other sessions. Memories are the only sharing point.
- **Default access.** Memory mounts default to read-only. Read-write is explicit and audited.
- **Snapshot mounts.** Mount config is captured at session creation. Archiving a Memory after that surfaces an error rather than disappearing files.
- **Source-backed Memories are read-only.** Memories synced from GitHub/Git cannot be mounted read-write; their contents are replaced atomically on sync.
- **Indexes are not surfaces.** Vector search or embeddings are an index *over* a surface, not a new surface. Surfaces are addressable storage.

## What is *not* Memory

These exist in Everruns but live outside this model:

- **Transcripts and events**: the conversation history is the session's append-only log, not a memory surface.
- **Sandboxes**: managed compute environments (`knowledge/runtime-resources/session-sandbox.md`) are a separate primitive from storage.
- **Knowledge Bases** (`knowledge/runtime-resources/knowledge-bases.md`), curated entries with stable citation IDs, agent reads via `search_knowledge`. Likely folds into a future "structured" surface of Memory; today it stays separate.

## Mapping to other systems

| Concept                       | Everruns          | Claude Managed Agents | Letta / MemGPT          |
|------------------------------|-------------------|-----------------------|-------------------------|
| Per-run scratch              | Workspace         | Memory Tool           | Working memory          |
| Durable named store          | Memory            | Memory Store          | Archival memory         |
| Mount access control         | RO / RW per mount | RO / RW per attach    | n/a                     |
| Background consolidation     |, | Dreaming              | Reflection              |
| Multi-surface (files/tables) | Files today; more planned | Files only       | Text blocks             |

## Further reading

- [Agent and user memory](/features/memory-scopes/), scoped memory mounts, access, and privacy defaults
- [`knowledge/runtime-resources/memory.md`](https://github.com/everruns/everruns/blob/main/knowledge/runtime-resources/memory.md), durable design intent for the Memory tier
- [`knowledge/runtime-resources/workspace.md`](https://github.com/everruns/everruns/blob/main/knowledge/runtime-resources/workspace.md), Workspace specification (file surface, mount point, git VCS)
- [`knowledge/runtime-resources/knowledge-bases.md`](https://github.com/everruns/everruns/blob/main/knowledge/runtime-resources/knowledge-bases.md), curated org knowledge, the curation-first sibling of Memory
