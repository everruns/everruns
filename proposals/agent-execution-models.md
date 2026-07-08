# Agent Execution Models — Unification and Next Steps

Status: proposal. Source analysis from a SOTA review (July 2026) of MCP tasks
(SEP-1686 → SEP-2663 extension), A2A v1.0, Claude Code, and the OpenAI Agents
SDK, against our session task registry (`specs/session-tasks.md`).

## Where we stand

Everruns already has the piece most of the ecosystem is still building: one
task registry unifying subagents, external A2A agents, background tool runs,
and monitors — one state machine, generic lifecycle tools (`list_tasks`,
`get_task`, `message_task`, `cancel_task`, `wait_task`), `task.*` events,
durability (heartbeats, attempt fencing, reaper, TTL), and one result
convention. MCP's task extension is converging on a subset of this; its known
gaps (retry semantics, result expiry) are already solved here.

The lagging part is the **spawn surface** and four capabilities the ecosystem
has converged on.

## Unification plan

1. **One `spawn_agent` for all agent delegation.** `spawn_subagent`,
   `start_agent_handoff`, and `spawn_agent` share one shape: instructions +
   target + mode → `task_id`. Merge them behind a `target` discriminator:
   `subagent` (same agent, child session), `agent` (configured Agent id — the
   handoff gate, `required_connections` intact), `external_a2a` (configured
   external agent). Handoff gets its own task kind (today it borrows
   `TASK_KIND_SUBAGENT`) and background mode for free — its stated blocker
   (no durable lifecycle updater) is gone since the registry landed.
   Keep `spawn_background` separate: tool-wrapping, not delegation; a union
   schema would degrade tool-call accuracy for no gain.
2. **Structured results.** Optional `result_schema` (JSON Schema) on spawn;
   see "Deterministic subagent → host communication" below.
3. **Mid-turn wake delivery.** Wake-ups today land between turns; inject task
   completions into the running parent loop (the specced-but-unbuilt subagent
   Phase 1b "steering messages").
4. **Push webhooks on task transitions** at the registry level (A2A v1.0
   parity; explicit v1 out-of-scope today). Prerequisite for inbound A2A —
   our task states map ~1:1 onto A2A task states.
5. **Deterministic orchestration above tasks.** Code-defined fan-out /
   pipeline / join running on the durable execution engine, spawning tasks as
   steps. Coordination moves out of the parent's context window; also the
   sanctioned answer to unbounded nesting depth.
6. **Per-task budgets.** Token/cost ceiling on the task record, enforced by
   executors; `Sealed` already exists as the exhaustion terminal state.

Non-goals: merging tasks with leased resources (different lifecycles), and
merging background tool runs into agent delegation.

## New requirements

### 1. Nested subagents (depth > 1)

Replace the boolean nesting guard (`parent_session_id` → hard `ToolError`)
with governed depth:

- `depth` derived from the parent chain; `max_subagent_depth` config
  (default 2), org/agent-cappable. Creation-time chain walk is bounded by the
  cap.
- **Shared budget pool, not per-level budgets:** the root session owns the
  budget; every descendant spawn debits the same pool. Exhaustion seals
  (`Sealed`), which is already terminal and non-retryable.
- Breadth + total caps per root (max concurrent descendant tasks; total
  descendant count backstop), mirroring the existing 6 h watcher cap.
- Add `root_session_id` (denormalized) to sessions/tasks so
  `GET /v1/tasks` can aggregate a whole delegation tree and the UI can render
  it as one unit.
- Wake-ups need no change: each child wakes its own parent; completion
  bubbles level by level.

### 2. Detached sessions (fan-out into peer sessions)

A Codex-"new thread" primitive: spawn an **independent top-level session**,
not a child bounded by the parent's lifecycle.

- Surface: `lifetime: "detached"` on the unified `spawn_agent` (default
  `linked` = today's child semantics). No new tool.
- Detached sessions get `parent_session_id = NULL` (they are peers), with
  lineage recorded separately — same modeling decision as forking
  (`specs/forking-sessions.md`), which already distinguishes fork lineage
  from subagent nesting.
- **Naming/goal:** `title` (exists on Session) set from the spawn's `name`;
  add an explicit `goal` field surfaced in session lists and to the agent
  itself (system-prompt visible), rather than smuggling it into the first
  message.
- Seeding options: `fresh` (blank), `fork` (reuse forking-sessions copy:
  history + workspace + KV), `workspace` (files only).
- Visibility without ownership: still create a task record
  (kind `session`, `wake_policy: silent` by default) so fan-out shows in the
  activity rail; the session outlives the record and its own lifecycle is its
  own. Cancel-task detaches tracking; it does not kill the session.
- Interplay with nesting: detached sessions are top-level, so they can spawn
  subagents under the current rule already — fan-out plus one level of
  nesting covers most trees even before item 1 lands.

### 3. Reuse and visualization of existing background machinery

Everything above is registry-native: new kinds/params on `session_tasks`,
not new mechanisms. Executors, wake policies, reaper, fencing, TTL, events,
and the generic tools apply unchanged.

Visualization: the specced session activity rail plus a cross-session
**Work view** — `GET /v1/tasks` grouped by `root_session_id`, rendered as a
tree (kind icon, state badge, progress, live `state_detail`), with the task
detail view tailing output. Chips render purely from `task.updated`
snapshots, so new kinds appear with no frontend work.

### 4. Deterministic subagent → host communication

Three layers, all validated at the tool-call boundary (the model retries on
schema mismatch — prose parsing never does):

- **Result contract:** `result_schema` on spawn. When set, the child gets an
  auto-injected `report_result` tool whose input schema *is* the declared
  schema; calling it writes the validated object to
  `/.tasks/{task_id}/result.json` and is the only way the task reaches
  `succeeded`. "Last assistant message" stays the fallback when no schema is
  declared.
- **Typed interim messages:** optional `message_schema` on spawn; the child
  gets `report_progress` for schema-validated `data` parts on the existing
  task message channel (`TaskMessage.content` already supports data parts).
  Delivered to the parent per `wake_policy` (`on_activity` → steering
  message carrying the machine payload).
- **Typed artifacts:** already exist (`artifacts: [{name, type, path|url}]`);
  no change.

This makes the parent-facing contract deterministic without constraining how
the child works internally.

## Sequencing

1. Spawn unification + handoff task kind (small, spec-first; enables the rest).
2. Deterministic results (`result_schema` / `report_result`).
3. Detached sessions with `goal` + seeding.
4. Governed nesting (depth, shared budget, `root_session_id`).
5. Mid-turn wake delivery.
6. Work view; webhooks; orchestration primitive.
