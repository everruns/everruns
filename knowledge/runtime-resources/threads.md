---
type: Specification
title: "Threads: How Agents Hand Off Work"
description: "How an agent hands work to another agent or to a copy of itself: two ways to start work (Spawn agents for one-off calls, Threads for work that is followed and continued), directional communication (parent, sibling), the Coordinator behaviour, shared files for a run, harness defaults that never offer both, and how subagents, agent hand-off and coordination fold into it."
tags:
  - everruns
  - runtime-resources
  - delegation
---
# Threads: How Agents Hand Off Work

Status: approved 2026-11-25 (decisions: thread is a role and session the
object; two kinds; sibling communication off by default; `/shared` for the
run). Implemented in part, see [Implementation status](#implementation-status).
Supersedes the delegation parts of [Subagents](subagents.md),
[Agent Handoff](agent-handoff.md), [Agents That Use Other Agents](agents-using-agents.md)
and [Coordination](coordination.md); those keep the executor details.

Source: [`delegation.rs`](../../crates/contracts/src/capability/delegation.rs)
(ids and the retired-id rewrite),
[`spawn_agents.rs`](../../crates/capabilities/src/capabilities/spawn_agents.rs),
[`threads.rs`](../../crates/capabilities/src/capabilities/threads.rs),
[`threads_parent.rs`](../../crates/capabilities/src/capabilities/threads_parent.rs),
[`sibling_communication.rs`](../../crates/capabilities/src/capabilities/sibling_communication.rs),
the [`threads_capability`](../../crates/server/migrations/20261125004823_threads_capability.sql)
migration, and the Framework's
[`members.rs`](../../crates/everruns/src/agent/members.rs) and
[`threads.rs`](../../crates/everruns/src/threads.rs).

## Abstract

A **thread** is a conversation an agent opens with another agent, or with a
copy of itself, to get one piece of work done. It reports back and can be
continued. A **spawned agent** is lighter: one call, one result, then it is
done. Today three features start work in another session, each with its own
tool, allow list, rules and screen: subagents (`spawn_agent` to itself), agent
hand-off (`spawn_agent` to a member) and coordination (`start_thread`).
Underneath all three is the same thing: a session with a parent, a task, and a
wake-up when the task changes. This design keeps that mechanism and gives it
two clear front doors that are never offered together, one way to talk to the
parent, and one set of rules:

| Capability | What it gives | Who adds it |
|---|---|---|
| **Spawn agents** | One-off calls to another agent or a copy of itself: brief in, result out | The agent; default only in the Coding harness |
| **Threads** | Open, follow and continue threads | The agent; never on by default |
| **Parent communication** | A child talks to the agent that started it | The host, on every spawned agent and thread |
| **Sibling communication** | Children of one parent message each other | The agent, opt-in |
| **Coordinator** | Behave as a coordinator | The agent; needs Threads |

An agent has Spawn agents or Threads, never both: saving both is refused, and a
harness default is dropped when the agent adds the other one.

## Problem

Evidence from the review of main at f063e04 plus #5640 (2026-11-24):

1. **Three tools for one intent.** The scheduled Housekeeping agent had
   coordination and the Agents capability. It called `spawn_agent` with an
   invalid target 36 times instead of `start_thread`. #5640 hides the Agents
   capability's `spawn_agent` from coordinators, but the Worker harness preset
   (`crates/contracts/src/capability/presets.rs`) adds `subagents` to every
   Worker-level harness, so every coordinator (Platform Chat included) still
   sees `spawn_agent` next to `start_thread`.
2. **Two lists of who may work:** coordination `workers` (agent ids, `self`,
   `any`) and Agents `targets`/`members` (names). #5640 lets members work
   threads, so a worker now comes from three sources.
3. **Different rules per path.** Nesting: subagents nest to depth 2, hand-offs
   start only from a run's first session, coordination threads never nest.
   Asking the person: approval cards routed to the first session, or
   `ask_decision` relayed by the coordinator. Finishing: `report_result`, or
   `complete_assignment`. Follow-ups: `message_task`, or `message_thread`.
   Copies of files: hand-off members only.
4. **Talking contradicts the rule.** The rule (user, 2026-10-08) is that
   threads talk only through the coordinator, yet session creation attaches
   `run_conversation` (`message_agent`) to every child session
   (`crates/server/src/domains/sessions/service/create.rs`), threads included.
5. **Shared folder skips the parent.** `/team` is mounted in every child
   session but in a first session only when its agent has `agent_handoff`
   (`team_folder_owner` in `domains/session_files/memory_mounts/mod.rs`), so
   Platform Chat cannot open its own threads' folder. Writes are whole-file:
   two agents writing one file lose an edit silently.
6. **Overloaded words.** "Thread" also names side chats ("Threads of this
   chat"); "Agents" is a capability, two tabs and the entities; four
   capabilities show a tool called "Spawn Agent"; catalog groups disagree.
7. **Docs cannot answer "which one".** No Platform page for threads, no
   comparison page, a stale foreman how-to (still requires `harness_id`).

## Goals

- The model never sees two ways to do the same hand-off: an agent has either
  Spawn agents or Threads.
- Both real use cases stay first class: a quick one-off call (fan-out,
  a specialist answering one question) and followed, continued work.
- A person picks capabilities by what the agent should do, not by mechanism.
- One set of rules for every thread: who works it, what it knows, nesting,
  files, approvals, credentials, cost.
- Agents in one run share files safely.
- No new stored object: a thread is a session, as today.

## Non-goals

- Shared live sandboxes. Parallel writers in one machine overwrite each
  other's files and processes; code work stays on a fork plus a git branch.
- Rooms, schedules, triggers and background commands. They run work later or
  elsewhere but open no thread (see "Not part of this").
- External agents (A2A, AG-UI) in the Threads list. Planned as a follow-up;
  they keep `spawn_agent`-style targets until then (see "Open questions").

## Vocabulary

| Word | Meaning | In storage |
|---|---|---|
| Session | One agent, one history, its own files and sandbox. The public API object. | `sessions` |
| Thread | The role a session plays when another agent opened it to do work that is followed and continued. Like "side chat", a role, not a second object. | session with `parent_session_id`, task kind `assignment` |
| Spawned agent | A session another agent started for one call: brief in, result out, done. | session with `parent_session_id`, task kind `subagent` |
| Child | A thread or a spawned agent, when the rule is the same for both. | session with `parent_session_id` |
| Parent | The session that opened a thread. | `parent_session_id` |
| Run | A conversation and every thread under it, at any depth. Cost, approvals and shared files belong to it. | `root_session_id` |
| Piece of work | One brief or follow-up a thread works on. Internal; the model and the person see thread status. | `session_tasks`, kind `assignment` |

Retired words, in UI, docs, prompts and tool descriptions: subagent,
hand-off, member, team, worker, assignment. Code may keep
internal names where renaming buys nothing.

Side chats stop being called threads: the chat switcher's "Threads of this
chat" becomes "Chats". Thread tool descriptions say "not a Slack or room
thread", for agents that also work in Slack or rooms.

## Model

```
conversation (parent) ── start_thread / message_thread ──▶ thread ─┐
       ▲                                                          │
       └──────── report_progress / ask_parent / finish ────┘
thread ◀── message_thread ──▶ thread          (only with Sibling communication)
every session of the run ── /shared ── versioned files
```

### Two ways to start work

| | Spawn agents | Threads |
|---|---|---|
| For | A quick call: fan-out, a second opinion, a specialist answering one question | Work someone follows: a change, a report, research the person opens and reviews |
| Tools | 1: `spawn_agent` | 5: `start_thread`, `message_thread`, `list_threads`, `get_thread`, `close_thread` |
| After the result | Done; a new call is a new spawned agent | Takes follow-ups in the same thread |
| Progress | Optional `report_progress` | Checklist the person sees live |
| People see it | A row under the step that spawned it | A thread in Chat's side panel, with its own page |
| Default | Coding harness | Never; added by the agent (Platform Chat has it) |

Both are configured the same way: the same `agents`, `self`, `any_agent` and
limit fields, so moving an agent from one to the other keeps its list.

### Capability: Spawn agents (`spawn_agents`)

One-off calls. Replaces `subagents` and `agent_handoff`'s `spawn_agent`
target. Config is the same shape as Threads (`self`, `agents`, `any_agent`,
`max_open`, `max_total`, `max_depth`), with `self: true` by default.

| Tool | Arguments | Result |
|---|---|---|
| `spawn_agent` | `name`, `instructions`, `agent` (name or id; default this agent), `wait` (bool, default false), `result_schema`, `model` and `blueprint` (only when this agent runs it), `sandbox: fork` | `task_id`, or the result when `wait` |

While it runs, the parent can read, message or cancel it with the existing
task tools (`get_task`, `message_task`, `cancel_task`, `wait_task`), which
come with this capability. `target.type`, `mode: invite` and `lifetime:
detached` are gone (invite and fork move out, see "Not part of this"). The
executor is today's subagent and hand-off executor, unchanged underneath.

### Capability: Threads (`threads`)

Opens and steers threads. Replaces `subagents`, `agent_handoff` and the tools
and `workers` list of `coordination`.

Config:

```json
{
  "agents": [{ "agent_id": "agent_..." }],
  "self": true,
  "any_agent": false,
  "max_open": 8,
  "max_total": 100,
  "max_depth": 2
}
```

- `self`: this agent may work threads. Default `true`.
- `agents`: other agents that may work threads. Each entry needs only
  `agent_id`; on save the platform fills `name` and `description` from the
  agent, as hand-off members do today. Optional per entry: `files: "copy"`
  (start with a copy of the parent's files), `required_connections`.
- `any_agent`: any agent the person may run (Platform Chat).
- Limits default to today's coordination limits (8 open, 100 total) and
  subagent depth (2).

Tools (names kept from coordination, so Platform Chat's prompts and evals
carry over):

| Tool | Arguments | Result |
|---|---|---|
| `start_thread` | `title`, `brief`, `agent` (name or id; default this agent), `wait` (bool, default false), `result_schema` (JSON Schema, optional) | `thread_id`, or the finished result when `wait` |
| `message_thread` | `thread_id`, `text`, `title` (names a new piece of work when the thread was idle) | ack |
| `list_threads` | `status` filter | threads with status, current piece of work, checklist summary; plus the agents that can work threads, with "used when" |
| `get_thread` | `thread_id` | pieces of work with checklists and results, last messages |
| `close_thread` | `thread_id`, `cancel` (stop work in progress), `reopen` | ack |

The system prompt contribution lists the agents that can work threads with
their descriptions (fixes the gap #5640 closed through `list_threads`) and
says when to wait and when not to.

The executor reuses today's coordination executor
(`crates/capabilities/src/capabilities/threads.rs`): `start_thread`
creates the session and an `assignment` task; `wait: true` waits on the
task like a foreground spawn; `result_schema` is checked at `finish`
like `report_result` today. Blueprints and helper model keys from
`spawn_agent` (`subagent_spawn_args.rs`, `subagent_helper_models.rs`) move
to `start_thread` as optional `model` and `blueprint` arguments, valid only
when this agent works the thread.

### Capability: Parent communication (`parent_communication`)

Added by the host to every child, thread or spawned agent alike; a spawned
agent uses `finish` for its result and rarely anything else.

Lets a sibling communication to its parent. The host attaches it to every session with
a parent, as it attaches `run_conversation` today; it shows in the catalog
but no one adds it. Replaces `update_checklist`, `complete_assignment`,
`ask_decision`, `report_to_coordinator`, `redirect_to_coordinator`,
`report_result`, `report_task_progress`.

| Tool | Arguments | Effect |
|---|---|---|
| `report_progress` | `steps` (checklist), `note` (optional) | updates the piece of work; a note wakes the parent only when marked important |
| `ask_parent` | `question`, `options` (optional, up to 4) | piece of work waits for input; parent is woken |
| `finish` | `outcome` (done, failed), `summary`, `links`, `result` (must match `result_schema` when set) | settles the piece of work; parent is woken |
| `hand_back` | `request` | posts to the parent: this belongs elsewhere |

A thread turn that ends without `finish` flags the piece of work as
needing attention, as the coordination listener does today
(`crates/server/src/listeners/coordination/mod.rs`, `settle_thread_turn`).

### Capability: Sibling communication (`sibling_communication`)

Lets threads of the same parent message each other directly. Off unless the
parent's agent has it. No new tools: inside a thread, `list_threads` and
`message_thread` also reach its siblings. The 200-message per-run cap and the
"from another agent, not the person" label (TM-AGENT-040) carry over from
`run_conversation`, which is removed.

### Capability: Coordinator (`coordinator`)

A system prompt: answer quick questions yourself, put real work in threads,
one thread per piece of work, follow-ups to the same thread, relay results,
close threads the person is done with, never answer a thread's question for
the person. Requires `threads`; the agent checks report Coordinator without
Threads as an error. It owns no tools and no state.

## Rules for every child (thread or spawned agent)

| Question | Rule |
|---|---|
| Who works it | The agent named in `start_thread` or `spawn_agent`, if `self`, `agents` or `any_agent` allows it; resolved as the person (TM-AUTHZ). It runs on its own harness, tools and model. |
| What it knows | Its brief, its agent's setup, the shared files. Never the parent's conversation. |
| States | working, waiting (asked), finished, failed, closed. A finished thread takes a follow-up and works again; a finished spawned agent is done. |
| Blocking | Not by default; the parent is woken on finish, fail or ask. `wait: true` blocks. |
| Nesting | A child may start children if its own agent has Spawn agents or Threads, up to `max_depth` counted from the top of the run. |
| Files | Own workspace and sandbox, plus `/shared`. `files: "copy"` seeds a copy of the parent's files. |
| Approvals | Always to the person, on the session at the top of the run (today's run requests). Never to an agent (TM-AGENT-039). |
| Questions | `ask_parent`; the parent answers or asks the person. |
| Credentials | Resolved for the person who started the run; never passed between sessions. |
| Cost | Per child and summed per run; the root spend pool applies to the whole run. |

## Shared files

Every session of a run sees one folder at `/shared`, the top session
included. It replaces `/team`.

- **Storage.** The existing run Memory (scope `run`, owned by the run's top
  session, `ensure_run_memory`). Mounted for every session with a
  `root_session_id`, and for a top session as soon as it opens its first
  thread. The `agent_handoff` check in `team_folder_owner` goes.
- **Versions.** Every write, move and delete records a version with the
  author session, as room files do (`room_file_versions`). Reuse that table
  shape for run Memories rather than a second one.
- **Safe writes.** A file tool write to `/shared` carries the version the
  session last read; if another session wrote since, the write fails with
  "changed by <thread> since you read it; read it again". Edit-in-place
  (`edit_file`) anchors on text and retries once on a fresh read.
- **People.** Shown as "Shared" in the top session's Files and as one folder
  on the Threads tab.
- **Lifetime.** As long as the run's top session; soft-deleted with it, never
  purged.
- **Next step (separate PR):** a run SQL database beside the session ones, for
  structured shared state (task lists, findings, best-of-N results).
- **Beyond one run:** organization Memories named in the agent's `memory`
  capability, as today; no new mechanism.

## Not part of this

| Feature | Change |
|---|---|
| Schedules | None. The same session continues later. |
| Triggers | None. The agent starts a new top session. |
| Background commands | `spawn_background` becomes `run_in_background`, says it runs shell commands, loses `schedule` (monitors become schedules with `wake_when`). Agreed 2026-11-24. |
| Invite | Leaves `spawn_agent`; becomes `invite_agent` with session participants. Agreed 2026-11-24. |
| Fork / detached | Leaves `spawn_agent`; stays the session fork action. |
| Session tasks tools | `list_tasks` and friends stay for background commands and monitors; thread tasks are hidden from them when Threads is present, so the model has one way to read a thread. |
| Rooms | None. |

## Storage and API

- **No new tables** except versions for run Memories (or a generalised
  `memory_file_versions`). One migration (`just new-migration
  threads_capability`) rewrites stored capability rows:
  `subagents` → `spawn_agents {self: true}`; `agent_handoff` →
  `spawn_agents {agents}`; `coordination` → `threads {self, agents from
  workers, any_agent}` plus `coordinator` (role worker rows are dropped:
  parent communication is automatic); `run_conversation` rows →
  `sibling_communication`. An agent with coordination keeps its hand-off
  members in `threads.agents` and loses `spawn_agents` (the Housekeeping
  case); otherwise subagents and hand-off merge into one `spawn_agents` row. Read paths accept the old ids as aliases for
  packages and exports made before the change.
- **Harness presets:** neither capability in Base, Conversation or Worker.
  The Coding harness gets `spawn_agents {self}` (parallel investigation is
  part of coding). Platform Chat seed: `threads {self, any_agent}` +
  `coordinator`. Bashkit Worker and Sandbox Worker agents that used subagents
  keep them through the migration, which writes `spawn_agents` on the agent
  when its harness gave `subagents` and the agent has no Threads.
- **Never both.** Validation refuses an agent with both `spawn_agents` and
  `threads`. When an agent adds Threads on a harness that gives Spawn agents,
  Threads supersedes it (the harness default is dropped for that agent), so a
  harness change never breaks a saved agent.
- **Tasks:** threads use kind `assignment`, spawned agents kind `subagent`
  (to another agent too). Old `agent_handoff` rows keep their executor until
  settled and stay readable; nothing is rewritten.
- **REST:** no new resource. A thread is a session; `GET /v1/sessions` gains a
  `parent_session_id` filter if missing. `GET /v1/platform/tasks`,
  `/v1/platform/sessions/{id}/run` and `/run-requests` are unchanged.
- **Work Chat:** `ChatFeatures.threads` and the consumer delegation check
  (`worker_link/consumer_delegation.rs`) read the `threads` capability and its
  agent list instead of coordination's `workers`.
- **CLI / MCP:** capability config through the existing agent commands; the
  CLI help, eval catalog and OpenAPI regenerate.
- **Agent packages:** `members = [...]` and `.agents/agents/` keep working and
  write `threads.agents`; export writes the new form.

## Data plane v2 / agentd

Threads and Parent communication are native capabilities (sessions, tasks), consistent
with the native-vs-injected rule. `crates/agentd/src/subagents.rs` already
runs children on any shard from the org's catalog and bounds them by the
capability's allow list; it reads the `threads` config instead of three. The
assignment lives in the parent's shard task state; a thread on another shard
reaches it through the gateway, and the thread's own turn events settle it
(`crates/agentd/src/thread_turns.rs`, the shard's counterpart of the server's
thread turn listener). A child session is never offered Threads or
Coordinator, on either data plane (`hidden_reason_capabilities`).

## Framework and serve

- `AgentBuilder::member(agent)` is replaced by `AgentBuilder::spawns(agent)`
  (Spawn agents) and `AgentBuilder::threads_with(agent)` (Threads); building
  both on one agent fails, as the platform refuses it. `member` is removed
  (pre-1.0, no compatibility).
- The facade's coordination runner (now `crates/everruns/src/threads.rs`)
  becomes the Threads runner; local profiles still refuse agents they do not
  host.
- serve: `#[agent]` `.member("name")` becomes `.spawns("name")` or
  `.threads_with("name")`.

## UI (mockups before building)

- Agent page: the Agents tab becomes the settings of whichever of Spawn
  agents or Threads the agent has: "Who can it hand work to" (this agent, a
  list, any agent) and limits. Coordinator is a normal capability toggle.
- Session page: one tab (name with the mockups) for the run's
  threads and spawned agents with status and cost, run cost, requests waiting
  on the person, replacing the Agents tab. Work keeps
  background commands and schedules.
- Chat side panel: unchanged list of threads; other background work folds
  under "Also running". Switcher: "Chats" instead of "Threads of this chat".
- A thread's page is headed "Thread opened by <parent>".

## Docs

- New Platform page "Hand off work with threads": vocabulary, the four
  capabilities, the rules, "when to use what".
- Capability pages for Threads, Parent communication, Sibling communication, Coordinator; Sub
  Agents and Agents pages redirect.
- Foreman how-to rewritten on Threads.
- Changelog entry (user-visible: one Threads capability, shared files).

## Security

- TM-AUTHZ: agents in the list resolve as the person; one not visible to the
  person is refused, as today.
- TM-AGENT-039: approvals and questions from tools are answered only by a
  person; `ask_parent` is a question to the parent, never an approval.
- TM-AGENT-040: messages from another thread are labelled as such.
- TM-DOS: open/total/depth limits, 20 follow-ups per piece of work before the
  parent must ask the person, 200 sibling messages per run, root spend
  pool.
- Shared files are written by other agents: content read from `/shared` is
  untrusted input like any file; versions record the author.

## Tests

- Capability resolution: old ids map to `spawn_agents`/`threads`/`coordinator`/
  `sibling_communication`; an agent with coordination + Agents + subagents
  yields one `threads` row and exposes no `spawn_agent` (regression for the
  Housekeeping loop); saving `spawn_agents` + `threads` is refused; a Coding
  agent that adds Threads loses the harness's Spawn agents.
- `start_thread` with `self`, an allowed agent by name and by id, a refused
  agent, `wait: true` returning the result, `result_schema` enforced at
  `finish`.
- Parent communication attached to every session with a parent and absent from top
  sessions; a turn ending without `finish` flags attention.
- Sibling communication off: siblings not listed and not reachable; on: reachable,
  capped, labelled.
- `/shared`: mounted in the top session and every thread; stale-version write
  refused; versions record authors.
- Migration: rows rewritten, packages with `members` still import.
- agentd: a thread on another shard starts, reports and settles.
- Live check (after merge, test org): Housekeeping-style coordinator hands a
  PR to a named agent; Platform Chat opens a thread; coding agent opens three
  parallel threads with `wait: false`.

## Rollout

One implementation PR (runtime, migration, API, CLI, MCP, Framework, serve,
agentd, docs, knowledge, changelog). UI follows after mockups are approved.
The run SQL database and external agents in the Threads list are follow-ups.

## Decisions

Approved by the user 2026-11-25.

1. **Word:** thread for the role and session for the object; side chats stop
   being called threads.
2. **Kinds of child:** two, Spawn agents and Threads, never on one agent;
   neither in base harnesses (user, 2026-11-24).
3. **Sibling communication:** off unless the parent's agent has it.
4. **Shared files:** `/shared` for the whole run, versioned, refusing silent
   overwrites; run database next.

## Implementation status

Done in the implementation PR (2026-11-25):

- `spawn_agents`, `threads`, `coordinator`, `sibling_communication` and the
  host-attached parent tools; `subagents`, `agent_handoff`, `coordination` and
  `run_conversation` are aliases rewritten by
  `canonicalize_delegation_capabilities` wherever capability lists resolve and
  on every write, and by the migration for stored rows (originals kept in
  `retired_delegation_capabilities`).
- `spawn_agent` takes `agent` and `wait`. Calls in the retired `target`/`mode`
  form are still accepted so recorded scripts and evals keep running;
  `mode: invite` is refused in favour of `invite_agent`.
- Never both: validation refuses an agent with Spawn agents and Threads; the
  Worker preset no longer carries subagents; Coding gets `spawn_agents
  {self}`; Platform Chat gets `threads {self, any_agent}` + `coordinator`.
- `/shared` replaces `/team`, mounted for a top session whose agent has Spawn
  agents or Threads, and for every child.
- Framework: `SpawnAgents`, `AgentBuilder::spawns`, `threads::{Threads,
  Coordinator, list}`, `AgentBuilder::threads_with`; serve `.spawns(name)`.

Not yet done (follow-ups):

- Versioned `/shared` writes with stale-write refusal.
- `start_thread` with `wait`, `result_schema`, `model` and `blueprint`.
- Sibling messages through `list_threads`/`message_thread` inside a thread:
  sibling communication still uses `message_agent` and
  `read_run_conversation`.
- A catalog entry for parent communication; UI renames and the run tab wait
  on approved mockups.

## Open questions

- External agents (A2A, AG-UI) as entries in the Threads list: they cannot
  take follow-ups the same way; may need `message_thread` to map to a new
  remote task.
- Whether `list_tasks` should disappear for agents without background
  commands.
- Name of the agent-page tab if Coordinator agents want it labelled
  differently from Threads.
