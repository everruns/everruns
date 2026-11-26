---
title: Memory
description: Give an agent durable memory across sessions, with typed entries for decisions, rules, facts, preferences and blocked items, kept as Markdown files under /memory.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `agent_memory` (alias `memory`) |
| **Category** | Storage |
| **Tools** | `remember`, `recall`, `forget`, `resolve`, `accept` |
| **Features** | `file_system` |
| **Risk** | Medium |

The Memory capability gives an agent memory that outlives a session. Each
memory is a folder of Markdown files under `/memory`, one folder per store. The
agent records typed entries with `remember`, finds them again with `recall`,
and sees what is open on a short memory card. Any file tool can read or edit
the same files. [How memory works](/features/memory-scopes/) explains the
stores, the entry format and who can see what.

It is part of the Conversation and Worker presets, so most agents have it
already.

## Tools

| Tool | What it does |
|---|---|
| `remember` | Records a `decision`, `rule`, `fact`, `preference`, `blocked` item or `note` in a store, with the person's exact words as the quote. Remembering the same kind and subject again replaces the entry. |
| `recall` | Searches entries and free-form files by words. Identifiers such as `everruns#5528` work. Filters by store and kind. |
| `resolve` | Closes a blocked item, with how it cleared. |
| `forget` | Deletes an entry. Its earlier versions stay in the memory's history. |
| `accept` | Moves a dreaming proposal into the store, in shared stores where proposals wait for review. |

## Configuration

Every store is a switch: `off`, `read`, or `read_write`. Leave a key out to keep
its default.

```json
{
  "ref": "agent_memory",
  "config": {
    "agent": "read_write",
    "team": "read_write",
    "parent": "read",
    "user": "read_write",
    "org": "read",
    "attach": [
      { "memory": "everruns-maintenance", "mode": "read_write" }
    ],
    "card": true
  }
}
```

| Field | Default | Store |
|---|---|---|
| `agent` | `read_write` | `/memory/agent`: this agent's own memory, shared by all its sessions |
| `team` | `read_write` | `/memory/team`: shared by a coordinator agent and every thread or agent it starts |
| `parent` | `read` | `/memory/parent`: the agent memory of the agent that started this one; read only |
| `user` | `read_write` | `/memory/user`: private to the person, in their own sessions only |
| `org` | `read` | `/memory/org`: the organization's memory |
| `attach[]` | none | `/memory/<name>`: named memories, each `read` or `read_write` |
| `card` | `true` | Keep the memory card in the agent's context |

A store appears only when it is switched on and the session has it: a session
with no person has no `/memory/user`, and a session no agent started has no
`/memory/parent`. Attaching a name creates that named memory if it does not exist yet; you can
also create one first on the **Memory** page or with `everruns memories create`.

The agent's instructions describe each store it has and what each entry kind
is for, so an agent with only `agent` and `team` is told about only those two.

## Migrating from the old Memory capability

The earlier `memory` capability copied organization memories into
`/workspace` at session start. Its configurations were converted: each mounted
memory with a store name became an `attach` entry and appears live at
`/memory/<name>`. Memories synced from GitHub or Git were retired; sync those
documents into [Knowledge](/capabilities/knowledge/) instead.
