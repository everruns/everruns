# Goal

Per-session long-running objective, inspired by Codex CLI's `/goal`. The user
sets a durable goal via `/goal <objective>`; the agent reads it back from the
session VFS each turn and treats it as standing context across many turns.
Lifecycle subcommands (show / pause / resume / clear) keep the goal
user-owned.

This spec covers **v1**: storage, system-prompt injection, and the `/goal`
lifecycle commands. **v2** (deferred): autonomous post-FinalAnswer validator
and continuation loop, plus the agent-owned progress journal.

## Concepts

A **goal** is a user-set objective bigger than one prompt but smaller than
an open-ended backlog. It persists across turns of the same session and
gives the agent durable scope it can return to whenever it would otherwise
ask "what next?".

A goal has:

- **objective** — free-form text describing what to accomplish (≤4_000
  chars, matching Codex's ceiling).
- **status** — one of `active`, `paused`, `completed`, `cleared`.
- **version** — monotonic counter, incremented on every write. Reserved
  for optimistic concurrency in v2.
- **set_at** / **updated_at** — RFC 3339 timestamps.

Goals are session-scoped. They do not span sessions, agents, or
organizations.

## Storage

Goal state lives in the session VFS at the file-store-relative path:

```
.everruns/{session_id}/goal.json
```

Agent-visible (under `/workspace`):

```
/workspace/.everruns/{session_id}/goal.json
```

### Why the VFS, not session metadata or a dedicated table

- **Agent-discoverable.** The agent can read the file with its existing
  file tools. No new "read goal" API to wire.
- **User-editable in place.** Any tool that exposes the VFS lets the user
  inspect or hand-edit the goal.
- **Composable with future capability state.** `.everruns/{session_id}/`
  reserves a namespace for sibling capability files (progress journal,
  memory, …) without per-feature placement decisions.
- **No DB migration.** Goal lifecycle is a thin layer over file
  read/write.

### Why session_id is in the path

The VFS is already per-session, so the `{session_id}` segment is
redundant inside that scope. We include it anyway for three reasons:

1. **Self-describing.** If the file is exported, snapshotted, surfaced
   in logs, or otherwise leaves the VFS context, the path identifies
   the session.
2. **Future-proofing.** Reserves a stable, per-session namespace for
   capability-owned state; siblings (`.everruns/{session_id}/journal.md`
   etc.) get the same prefix from day one.
3. **Convention parity.** Matches the prefixed-ID style used elsewhere
   in the codebase (`specs/id-schema.md`).

### Why `.everruns/`, not the VFS root

Dotfile-style hides the capability state from casual workspace listings
without making it inaccessible. Mirrors existing precedent
(`/workspace/.outputs/` from `gpt_image_gen`).

### Schema

`goal.json` is a single JSON object (pretty-printed for human edits):

```json
{
  "objective": "Finish the 0042 migration and keep tests green",
  "status": "active",
  "version": 3,
  "set_at": "2026-05-20T10:00:00Z",
  "updated_at": "2026-05-20T11:23:11Z"
}
```

Readers must ignore unknown fields so future versions can extend the
record without breaking older code paths. The Rust type lives in
`crates/core/src/capabilities/goal.rs` as `GoalRecord`.

## `GoalCapability`

Defined in `crates/core/src/capabilities/goal.rs`. ID: `"goal"`.

Contributes:

- The `/goal` system command (no tools, no MCP servers, no mounts).
- A dynamic `system_prompt_contribution()` that reads `goal.json` and
  injects active-goal context. When the goal is missing, paused,
  completed, or cleared, the contribution returns `None` — the agent
  only carries goal context while it is `active`.

The `Generic` harness opts into `goal` by default
(`crates/server/src/harnesses/generic.rs`).

### System-prompt injection

When the goal is `active`, the capability injects a `<capability
id="goal">` block containing:

- The objective text inline.
- The agent-visible path to `goal.json` so the agent can re-read the
  authoritative copy.
- A note that the goal is user-owned and `/goal` commands are the
  supported edit surface.

Injection is lazy: the capability does **not** stuff the goal into the
prompt at session creation. It is read on every system-prompt build,
ensuring the agent always sees the live state.

## `/goal` command

System command (`CommandSource::System`), dispatched in
`SessionCommandService::execute_goal`. Single optional argument; the
handler parses subcommands:

| Invocation | Effect |
|---|---|
| `/goal` | Show the current goal (or "no goal set"). |
| `/goal show` / `/goal status` | Same as `/goal`. |
| `/goal <text>` | Set the objective to `<text>`, status `active`. Replaces any prior goal. |
| `/goal pause` | Set status to `paused`. |
| `/goal resume` | Set status back to `active`. |
| `/goal clear` / `/goal cancel` / `/goal stop` | Set status to `cleared`. |

Returns a `CommandResult` (overlay-style, no chat message persisted).
`success: false` is reserved for usage errors that the UI surfaces inline
(e.g. unknown subcommand or objective too long). The 4_000-char limit on
new objectives is enforced server-side and matches Codex's ceiling.

### Notes on state transitions

- Setting a new objective on top of an existing goal **replaces**
  it, increments `version`, and preserves `set_at`.
- `pause`/`resume`/`clear` on a missing goal returns a friendly
  "no goal set" response, not an error.
- `clear` does not delete `goal.json`; it sets `status: "cleared"`. This
  keeps the audit trail and version counter monotonic. v2 may grow a
  separate `/goal forget` to delete the file outright.

## Out of scope (v1)

The following are intentionally **not** in v1 and will be added in
follow-up PRs:

- **Autonomous continuation loop.** Codex's `/goal` keeps the agent
  working across many turns without user input. v2 adds a goal-gate
  execution-phase step that runs after `FinalAnswer`, calls a cheap
  validator LLM, and either accepts the FinalAnswer or synthesizes a
  user-role continuation message inside the durable workflow (see
  `specs/durable-execution-engine.md` — the loop is checkpointed for
  free).
- **Progress journal.** `.everruns/{session_id}/goal-journal.md` for
  agent-written notes about what's been tried. v2 wires this into the
  validator so the agent doesn't repeat itself across iterations.
- **Iteration / budget caps.** A hard ceiling on how many auto-continued
  turns a single goal can drive before requiring user check-in, plus
  integration with `specs/budgeting.md` to auto-pause on budget
  exhaustion.
- **Pause-on-restart.** When a worker restarts mid-loop, sweep `active`
  goals with stale `updated_at` to `paused(reason: "worker_restart")`.

## Threat model touchpoints

- **User-owned state.** The goal is user-controlled. The agent **can**
  write to `goal.json` via normal file tools (we don't ACL it) but the
  capability's system prompt instructs the agent not to. Misbehaviour is
  contained the same way the rest of the agent loop is: tool-use audit,
  iteration caps (v2), and validator cross-check (v2). Choosing not to
  ACL the file keeps user hand-edits cheap; the integrity tradeoff is
  no worse than the existing tool-use trust model.
- **Prompt-injection content.** The objective text flows into the
  system prompt verbatim. The capability wraps it in a `<capability
  id="goal">` XML envelope per the existing
  `specs/xml-prompt-formatting.md` convention; downstream injection
  guards apply unchanged.

## Tests

- `crates/core/src/capabilities/goal.rs` — metadata, command schema,
  path helpers, record serialisation.
- `crates/server/src/domains/session_commands/service.rs` —
  subcommand parsing and `goal_show_result` rendering.

## References

- Codex CLI `/goal` docs (external):
  `developers.openai.com/codex/use-cases/follow-goals`,
  `developers.openai.com/codex/cli/slash-commands`.
- `specs/commands.md` — slash-command system this builds on.
- `specs/session-filesystem.md` — VFS layout and `/workspace` path
  convention.
- `specs/durable-execution-engine.md` — the substrate v2's continuation
  loop will run inside.
