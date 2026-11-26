---
type: Specification
title: "Memory Specification"
description: "Agent memory v2: stores under /memory (agent, team, parent, user, org, named), typed Markdown entries, the memory card, recall, kept versions and privacy."
tags:
  - everruns
  - runtime-resources
  - memory
---
# Memory Specification

Status: accepted (2026-11-26), replaces memory v1.

## Abstract

Memory is what an agent keeps between sessions. It is folders of files under
`/memory`, one folder per **store**. Typed **entries** (decisions, rules,
facts, preferences, blocked items, notes) are Markdown files in kind folders,
so an agent, a person, and any file tool read the same thing. The `agent_memory`
capability adds the tools that write and search entries and a short **memory
card** that keeps what is open in front of the agent.

The design was checked against a real failure: a coordinator agent recorded a
person's decision about a Dependabot PR in session-only key/value storage, and
the next run asked again. Memory v2 makes the durable place the obvious place.

## Goals

1. A decision a person made is followed in every later session that should see
   it, and the agent can show the words it rests on.
2. Who can read and write is the store's owner, not a path convention: agent,
   team, the parent agent, the person, the organization, or a named group.
3. Files first. No entries table the agent cannot see; the file is the entry.
4. Nothing is purged. Every write and delete is kept as a version.
5. Works the same in the framework engine and the platform: the engine runs
   over `SessionFileSystem`, the server routes `/memory/<store>` to durable rows.

Non-goals for v2: meaning-based (embedding) recall, a memory event trigger, and
dreaming that writes typed entries. Memory is not Knowledge: Knowledge indexes
documents for search, memory holds what agents learned and were told.

## Stores

Defined by `MemoryStoreScope` in `crates/contracts/src/runtime/memory.rs`.

| Store | Mount | Owner row | Default access |
|---|---|---|---|
| agent | `/memory/agent` | `memories` scope `agent`, `owner_agent_id` = the session's agent | read_write |
| team | `/memory/team` | scope `team`, `owner_agent_id` = the agent of the run's root session (the coordinator) | read_write |
| parent | `/memory/parent` | the parent session's agent's `agent` row; never created on its behalf | read |
| user | `/memory/user` | scope `user`, the session's person; private sessions only | read_write |
| org | `/memory/org` | scope `org`, name `org` | read |
| named | `/memory/<name>` | scope `org`, any other row whose name is a store name | per attachment |

Decisions:

- Team memory follows the coordinator agent, not the run, so a team keeps its
  memory across runs. Threads and spawned agents share it read-write by
  default because a worker that cannot write back cannot resolve a blocked item.
- Parent is the parent agent's own memory, read-only by default: a child
  follows its parent's decisions without changing them.
- Named memories replace v1's mounted organization memories. Their names are
  store names (`validate_store_name`), so `/memory/<name>` is unambiguous.
- `/memory/shared` (Platform Chat's harness memory) is unchanged.

Every store is a switch in the capability config (`AgentMemoryConfig`). The
stores a session has are the config intersected with the folders the host
mounted. The server mounts them in `MemoryMountRouter::resolve`
(`crates/server/src/domains/session_files/memory_mounts.rs`) when the agent or
its harness has `agent_memory`; without it, only the legacy agent and user
mounts.

## Entries

An entry is `/<kind folder>/<slug>.md` inside a store, with a `---` header
(kind, subject, status, occurred_at, recorded_at, recorded_by, source,
source_ref, source_url, resolved_at, resolution), the statement as the body,
and the quote as trailing `>` lines. `parse_entry_file` and `render_entry_file`
own the format; hand-written files with only `kind` parse with defaults.

- One subject holds one current entry per kind: the slug comes from the
  subject, so remembering again overwrites the file and history keeps the old
  one. Notes never replace each other.
- `blocked` items are the only resolvable kind.
- Proposals live under `/proposals/<kind folder>/` and stay off the card until
  `accept` moves them. Intended for consolidation into shared stores
  (`dreaming_writes_proposals`).
- Writes that would not parse are refused at the server (`check_entry_write`,
  `check_entry_content`); files that already exist and do not parse are listed
  with their error rather than hidden.

## Capability `agent_memory`

`crates/core/src/builtins/agent_memory.rs`, alias `memory`, in the Conversation
and Worker presets. Tools: `remember`, `recall`, `forget`, `resolve`, `accept`.

- Recall is lexical: words and identifiers scored over entries and free-form
  files, bounded by `MAX_FILES_SCANNED`.
- The card is a `conversation_context_update_contribution`: delivered only when
  it changes, so the cached system prompt stays stable. It names every enabled
  store with what it is for, and every entry kind with when to use it, then
  lists open blocked items and recent entries per store.
- Read-only and absent stores refuse writes with errors the model can act on.

## Versions

`memory_file_versions` (migration `20261126064500_memory_v2.sql`) keeps every
write and delete, filled by a trigger on `memory_files` so no write path can
skip it. Room memories keep their own versions and are skipped. Writers may
set `everruns.memory_actor` for `changed_by`. Forget deletes the file; its
versions stay.

## API, CLI, UI

- `GET /v1/platform/memory/stores` (`everruns memories stores`): every store the
  caller can see, with entry, blocked and proposal counts.
- `GET /v1/platform/memories/{id}/entries` (`everruns memories entries`).
- `GET /v1/platform/memories/{id}/history?path=` (`everruns memories history`).
- The `memories` CRUD and `/fs` routes stay as the store API; creating a memory
  makes a named memory. Source-backed (GitHub/Git) memories are refused.
- Console: **Memory** under Blocks, no feature flag. Store list grouped by who
  shares it; a store page with entries by kind and history; the capability
  editor sets each store switch, attachments, and the card.

## Privacy and threats

- A person's memory is visible only to that person, whatever their org role
  (`visible_to`, THREAT[TM-TENANT-013]); other callers get not-found.
- User memory is mounted only in the person's private sessions.
- Read-write shared stores let one session steer later sessions of other
  agents: the capability risk is Medium, and dreaming writes proposals, not
  entries, into shared stores.
- Quotes keep a person's words separate from the agent's inferences, so a
  later reader can tell which is which.

## Migration from v1

- The v1 `memory` capability (copy mounts into `/workspace`) is removed. Its
  agent and harness rows become `agent_memory` with each mounted memory that
  has a store name attached; `readwrite` stays read-write unless the memory is
  read-only.
- GitHub and Git memories are archived; their sync job is gone. Use Knowledge.
- The `memory` feature flag is removed.

## Testing

- Contracts: entry round trip, hand-written defaults, bad files, card render,
  store keys, config switches (`memory_tests.rs`).
- Core: stores as config intersected with mounts, replace by subject,
  read-only refusals, resolve, proposals, recall over files
  (`agent_memory_tests.rs`).
- Server: entry checks, private-memory visibility, store listing and routing.
- UI: store list, store page, capability editor tests.

## Open questions

- Meaning-based recall through the search drivers Knowledge uses.
- A `memory` trigger source so a change can wake an agent.
- Dreaming writing typed entries (direct in agent and user stores, proposals in
  shared ones) instead of `/memory/user/learned` notes.
- Erasing a secret someone remembered by mistake: versions are never purged,
  so this needs an explicit admin path.
