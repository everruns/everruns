---
type: Specification
title: "Platform Chat v2"
description: "Rebuild Platform Chat as one Bashkit shell over a single namespace: the everruns CLI, read-only docs, and a writable shared memory folder."
tags:
  - everruns
  - harnesses
  - platform-chat
  - bashkit
  - memory
---
# Platform Chat v2

Status: proposed. Not implemented. Supersedes nothing until the acceptance bar
in [Acceptance](#acceptance) is met against the v1 harness.

## Abstract

Platform Chat v1 is an assistant with three bespoke tools (`discover`,
`query`, `execute`) that run bash *without* a filesystem, plus a ~4 KB system
prompt that teaches the model rules the tools could have carried themselves.
v2 keeps the product (a conversational operator surface for the platform) and
replaces the mechanism with one Bashkit shell over one namespace:

```text
/workspace            scratch, per session, ephemeral
/workspace/docs       product documentation, read-only
/memory               shared operator memory, read-write, durable
everruns <noun> <verb>  the control plane, as a CLI builtin
```

Nothing above is new machinery. `bashkit_shell`, the `everruns` command tree,
the virtual docs mount, and Memory all exist. v2 is mostly *wiring*, plus one
real gap: a Memory mount that is live rather than a snapshot.

## Why v1 needs replacing

| v1 property | Cost |
|---|---|
| Three tools whose only difference is which builtins are registered | `query` vs `execute` is a toolset split the model must reason about, not a permission boundary. Authorization already lives in the command execution path (THREAT[TM-AGENT-017]). |
| No filesystem in the scripting environment | The prompt has to forbid redirection ("never write intermediate output to `/tmp`", "keep JSON in shell variables"), which costs tokens and loses multi-step work. |
| Flat namespace of 291 commands | `--help` is unaffordable, so `discover` exists to rank operations, and the prompt spends a paragraph teaching callers not to over-discover. |
| Deliberately no `session_file_system` | The embedded `/docs` mount the `platform` capability declares cannot be browsed by file tools, and `memory` is excluded outright, "revisit together with a VFS decision". This is that decision. |
| Rules as prompt prose | Preflight discipline, the five authoritative views, credential handling and confirmation policy are all prompt text. They drift from the code they describe and are re-sent every turn. |

## Design

### 1. One shell, one namespace

Replace the `platform` capability's three tools with `bashkit_shell` plus the
`everruns` CLI builtin, over the session filesystem.

Bashkit already supports this exactly. `integrations/bashkit/src/lib.rs`
installs an `EverrunsBuiltin` into the interpreter whenever a
`CliCommandSourceHandle` is present on the tool context
(`install_cli_tree`). Today **no host inserts that handle**, so the builtin is
dead code on the server and the command tree is reachable only through
`cli_tree::rewrite` inside the filesystem-less `ScriptedTool`. The v2 delta on
this axis is a server-side `CliCommandSource` whose `dispatch` actually runs a
domain command with the caller's identity, inserted into the tool context for
sessions carrying the capability.

Consequences worth stating:

* Pipes, `jq`, loops, and redirection compose over *both* platform output and
  files, in one language. `everruns agents list | jq -r '.data[].id' > /workspace/ids`
  becomes legal and useful.
* `query` / `execute` collapse. The read-only/mutating distinction stops being
  a tool boundary and becomes a per-command property: `CommandDescriptor`
  already carries `read_only`, and the CLI source consults it for policy and
  approval, which is where the fact belongs.
* `discover` survives only as `everruns search <phrase>`: one surface, and the
  ranking logic it already has. Tree `--help` is bounded by shape (root lists
  nouns, node lists children, leaf renders its flags), so browsing is
  affordable where a flat list of 291 names is not.

### 2. Management is the `everruns` CLI

The grammar, help rendering, and route declarations exist
(`integrations/bashkit/src/cli.rs`, `Command::cli()`). The gap is coverage:
**51 of 291 commands declare a `CliRoute`, across 4 domains of ~40**
(`agents`, `sessions`, `skills`, `mcp-servers`). Everything a v1 operator
thread touches through `execute` today, harnesses, capabilities, plugins,
models, connections, triggers, schedules, memories, knowledge indexes, budgets,
must be routed before v2 can claim parity.

Membership stays opt-in by construction (a command joins the tree only by
declaring a route), so this is deliberate per-domain work, not a sweep. Node
descriptions (`NODE_ABOUT`) grow with it; a node without one renders
"agents, agents commands" and teaches nothing.

### 3. Docs are a read-only folder

Already built: `platform_docs.rs` embeds the repo `docs/` tree at compile time
and the `platform` capability mounts it read-only at `/docs` as a
`MountSource::Virtual`, served from the per-session `VirtualMountRegistry`
without touching `session_files`. v2 keeps it and, for the first time, gives
the model tools that can read it: `grep -r pattern /workspace/docs`,
`ls`, `cat`.

Later (not v2 P0): make docs a source-backed Memory (`source_type: github`,
`root_folder: docs`) so documentation tracks the published docs site without a
redeploy. The compile-time embed is a build-order convenience, not a product
decision.

### 4. Memory is a writable folder

Mount a durable Memory read-write at `/memory`. The model writes notes,
conventions, and org context there; the next thread reads them.

This is where v1's exclusion note pointed ("Memory is driven by `mounts[]`
naming concrete per-org `mem_` IDs that a built-in definition cannot know").
The answer is that Platform Chat's memory is *server-managed*, like agent and
user memory already are: the session service resolves or creates a well-known
org-scoped Memory for the chat surface and mounts it, exactly as
`collect_scoped_memory_mounts` does for `/memory/agent` and `/memory/user`. No
built-in definition has to know an ID.

## The shared-memory problem

> All instances of Platform Chat v2 must work against the same memory.

### What breaks today

`memory.md` specifies write-through: "Read-write mounted paths write through to
`memory_files`." The implementation does not do this. `memory_row_to_mount`
calls `list_all_memory_files` and builds a `MountSource::directory(...)`, and
`apply_single_mount` copies a `Directory` source into `session_files` rows.
A Memory mount is therefore a **snapshot taken at session creation**, and every
write lands in that session's private copy and dies with it.

For Platform Chat this is fatal by construction: each UI thread is its own
session, so N threads are N divergent copies. Platform Chat v1 also has no
agent, so `/memory/agent` never mounts at all; only `/memory/user` does.

### Fix 1: make the mount live (required)

Add a mount source resolved at access time rather than copy time, and teach the
session file service to route reads and writes under a memory mount to
`memory_files`:

* **Read**: resolve through the mount, merged into listings the way the
  snapshot is merged today. `/memory/user` redaction (`redact_user_memory_files`)
  already exists and keeps working.
* **Write**: write through to `memory_files` with the `content_hash`
  precondition the spec already mandates. A concurrent write fails the
  compare-and-set and returns a stale-edit error naming the current hash; the
  model re-reads and retries. Files are independent rows, so two threads
  editing different notes never contend.
* **Delete / archive**: unchanged, `memory_archived` / `memory_deleted` per spec.

This is the load-bearing change. Everything below is convention on top of it.

### Fix 2: shard the write path (recommended)

Compare-and-set is correct but a chat surface should rarely need it. Structure
the folder so concurrent threads do not write the same file:

```text
/memory/README.md          conventions, read-only to the model
/memory/INDEX.md           curated titles, one line per note
/memory/notes/<topic>.md   curated, stable, rarely rewritten
/memory/inbox/<session>.md append-only, one file per session, never contended
```

A thread appends observations to its own `inbox/<session_id>.md`; nothing else
writes that path, so lost updates are impossible without any locking. A
periodic compaction agent (an ordinary Agent Trigger, which v2 can create for
itself) folds the inbox into `notes/` and refreshes `INDEX.md`. That is a
single-writer batch job, so the curated tree has one writer by construction.

This also generalizes past the hosted server. When "the memory folder" is a
real host directory shared by several processes, per-writer files are the only
coordination that survives without a lock manager.

### Scope: shared and private, both

Shared and private memory are not alternatives. The runtime already mounts two
memories side by side at sibling paths (`/memory/agent` and `/memory/user`),
and the privacy boundary for the private one is already built and enforced.
Platform Chat v2 takes both:

```text
/memory/shared/   one memory per (org, surface). Every operator, every thread.
/memory/user/     private to the session owner. Existing path, existing rules.
```

`/memory/user` is reused verbatim, not re-invented. `is_user_memory_path` makes
the path reserved, `verify_session` sets `user_memory_allowed` only when the
caller is the session's resolved owner (or internal), and reads, recursive
listings, and grep all redact it otherwise, with the storage layer excluding it
before matching as defense in depth. Nothing about that changes.

Siblings, not overlays. The memory spec rejects overlapping mount paths, so
neither scope shadows the other on the filesystem. Precedence is resolved where
it is legible instead: the disclosure block (Fix 3) merges both indexes,
labels each entry with its scope, and lets a private note win a title collision.

**Writes default to private; sharing is an explicit act.** A note goes to
`/memory/user` unless the user asks for it to be shared, because promotion is
the outward-facing direction and is not cleanly reversible: once an observation
is in `/memory/shared` it has been read by other people's threads. This is the
same split yolop draws between `MEMORY.md` (per user, never committed) and
`AGENTS.md` (per project, reviewed).

The inbox sharding of Fix 2 applies to both, for different reasons: the shared
tree has many concurrent writers, and the private tree still has one user's
several concurrent threads.

Resolving the shared memory needs no migration and no new scope. `memories`
already enforces `UNIQUE(org_id, name)` for live rows, so a reserved name
(`platform-chat`) is enough; the session service resolves or creates it the way
`get_or_create_scoped_memory` already resolves the agent and user ones. Only a
lookup-by-name is missing from the repository, which today offers
`get_memory_by_scope_owner` and `list_memories`.

There is a tempting shortcut: back v2 with a real Agent instead of a bare
harness, and `/memory/agent` becomes the shared surface memory for free. It is
rejected, because `org_init` deliberately does not auto-seed agents (they are
examples adopted on demand, which is what keeps duplicates out), so an
auto-provisioned built-in agent per org would reopen that. The reserved-name org
memory gets the same result without touching that decision.

**The cross-user risk is the shared tree, and it is the reason for the default.**
Anything in `/memory/shared` is written by one operator and read as context by
every other operator's threads, which is a persistence channel for prompt
injection and an accidental-disclosure channel for whatever a user pasted into
a conversation. Shared notes are data, never instructions; secrets never go in
either tree; and a promotion is a user-visible act, not a model's judgment call.

### Fix 3: disclose the index, not the folder (recommended)

Borrowed from yolop's `memory` spec: inject only note **titles** into the
system prompt each turn, with ids and dates, and let the model read bodies on
demand. Prompt cost stays flat as memory grows, the opposite of injecting the
folder. Here the disclosure block is generated from `/memory/INDEX.md`, which
the curator owns.

### Rejected alternatives

| Option | Why not |
|---|---|
| One shared Platform Chat session per user or org | Threads are a shipped product feature, and one session serializes every operator interaction. |
| Exclusive writer lease per Memory | Too coarse: blocks a second thread from writing an unrelated note. Right answer for a sandbox, wrong one for chat. |
| Memory as tools only (`remember` / `recall` / `forget`), no folder | Atomic and simple, but gives up the folder the requirement asks for and the shell composition that makes v2 worth building. The structured-note *convention* is worth keeping; the tool-only surface is not. |
| Last-write-wins with no precondition | Silent data loss on the exact interleaving a multi-thread operator surface produces. |

### Security note

A writable memory folder is a persistence channel for prompt injection: a note
written during one thread is read as context by every later thread, including
threads belonging to other users of the org. Memory content is **data, not
instructions**, and the prompt must frame it that way. Secrets never go in it,
the existing credential rule (binding + write-only setup form) is unchanged and
now also covers `/memory`.

## What Yolop contributes

v2 is the natural place to narrow the everruns/yolop gap, because yolop already
solved several of these problems in the same shape.

* **CLI over bash as the control surface.** Yolop administers extensions and
  session coordination through `yolop <noun> <verb>` invoked from the ordinary
  Bash tool rather than through bespoke model tools. That is the identical
  pattern to `everruns <noun> <verb>`, and the tree implementation in
  `integrations/bashkit/src/cli.rs` is already host-neutral (`CliCommandSource`,
  a host-owned `root()` token). Extracting it so yolop consumes the same crate
  is a concrete convergence item.
* **One fact, one home.** Yolop's system-prompt spec assigns every instruction
  an owner: how to call a tool lives in its `description()`, what to do about a
  result lives in the result, cross-cutting policy lives in the base prompt.
  Most of v1's system prompt is how-to for three tools and should move into
  their descriptions, tool results, and `--help`.
* **Progressive disclosure.** Discovery ("this exists") always rides; how-to
  waits until `tool_search` reveals the tool. Applied to v2: name the `everruns`
  root and its nouns always, defer flag-level guidance to `--help`.
* **Structured memory with title-only disclosure.** Fix 3 above.
* **Approval as a mechanism.** v1 says "always confirm before creating a
  harness or agent" in prose. Yolop's approval spec makes consequence a
  property of the action. With `read_only` already on every
  `CommandDescriptor`, confirmation can be enforced rather than requested.
* **Skills as the home for workflows.** Plugin wiring, agent creation
  preflight, and the five-view grounding check are procedures, which is what a
  skill is. Registry skills already mount at `/.agents/skills/`, so v2 can
  carry them as `skill:` refs instead of prompt paragraphs.

## Shape of the harness

v2 should be a *configuration*, not a code path: generic harness chain plus
`bashkit_shell`, `session_file_system`, `memory`, `skills`, and the chat-surface
capabilities v1 already carries (`human_intent`, `current_time`,
`message_metadata`, `prompt_caching`, `compaction`, `loop_detection`,
`tool_call_repair`, `stateless_todo_list`, `error_disclosure`). The bespoke
part shrinks to the CLI command source and the memory resolution.

That is also what makes it portable: the same composition running against a
real directory and a remote control plane is a terminal operator client, which
is the yolop-shaped end state.

## Acceptance

v2 ships when it passes, against the same models, the cases v1 is graded on:

* `knowledge/test-cases/agents/platform_chat/TC001`-`TC005`, create and run an
  agent, discover and execute, answer from embedded docs, create a scheduled
  agent, and ground plugin/connection state before creation.
* `knowledge/test-cases/ui/chats/`, the thread surface, agent creation at
  volume, and run relay.
* Two new cases v1 cannot express: a note written in thread A is readable in
  thread B, and two threads writing concurrently both survive.

## Rollout

1. **P0, wiring.** Server `CliCommandSource` with real dispatch; insert the
   handle into the tool context; seed `platform-chat-v2` beside v1 behind a
   feature flag, with `bashkit_shell` + `session_file_system` + the docs mount.
   Bar: TC001-TC005 pass.
2. **P1, coverage.** Route the remaining domains onto the tree; `everruns
   search`; a test that bounds rendered help size per node.
3. **P2, memory.** Live memory mount with write-through and CAS; server-managed
   Platform Chat Memory; folder conventions and the INDEX disclosure block.
4. **P3, discipline.** Move prompt prose into tool descriptions, results, and
   skills; approval driven by `read_only`; the curator trigger for the inbox.
5. **P4, convergence.** Extract the CLI tree into a host-neutral crate both
   everruns and yolop consume; flip the org default harness; retire v1.

## Open questions

* Does the live mount need a read cache, and if so what invalidates it, the
  Memory row's `updated_at` or nothing at all in the first cut?
* Should the curator be a built-in server task or an ordinary Agent Trigger the
  harness provisions for itself? The latter dogfoods the surface; the former
  does not depend on a model behaving.
* How much of the v1 preflight discipline survives contact with a real shell?
  Some of it exists because `query` could not keep state between calls.
