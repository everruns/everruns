---
type: Specification
title: "Agents That Use Other Agents"
description: "How one agent hands work to other agents: the Agents capability as the opt-in, members that are linked agents or agents created as part of this one, what members share and do not share, the package format with bundled agents in .agents/agents/, and the planned costs, approvals and trace views."
tags:
  - everruns
  - runtime-resources
  - agents
  - delegation
---
# Agents That Use Other Agents

Status: accepted (2026-11-02). Part 1 (members, created-here agents, package
format, Agents tab) is built. Approvals and questions on the first
session, the shared team folder and copy-of-files are built. The rest of
part 2 (run conversation) and part 3 (whole-run costs and traces)
follow. The mechanism underneath is [Agent Handoff](agent-handoff.md).

## Abstract

Everruns agents were flat: any agent could be configured to hand work to
another, but the setup was raw ids, nothing travelled in an export, and costs
were per session. An agent can now have **Agents**: other agents it hands work
to. A member is a real agent. It is either **linked** (any agent in the
organization) or **created here** (made as part of this agent: it is listed
under it on Agents home and travels in its export).

The platform imposes no names or roles. There is no "team", "lead" or "worker"
in the product: the agent page has an **Agents** tab, members are shown and
addressed by their own agent name, and "used when" is the member's own
description. "Team" and "member" are words for code and these notes only.

## Why this shape (state of the art, 2025-2026)

| System | What it teaches |
|---|---|
| Anthropic multi-agent research | A lead with parallel workers costs ~15x a chat; workers get a written brief and return references, not long replies |
| Claude Code subagents and agent teams | Members defined as files; teammates editing the same files overwrite each other; approvals bubble to the lead |
| OpenAI Agents SDK | "Agents as tools" (lead keeps control) vs "handoffs" (control moves): our background/foreground vs `invite` |
| Codex, Cursor, Copilot coding agents | Parallel coding work gets its own container or worktree; git is the merge point |
| Google/MIT scaling study (Dec 2025) | A central coordinator contained error amplification (4.4x vs 17x for independent agents); coordination helps parallel work and hurts sequential work |
| Cognition, "Don't build multi-agents" | Parallel writers make conflicting hidden decisions; parallel reading is safe |
| Warp Factories | Foreman routes triage, spec, implement and review; the branch and PR are the shared artifact; review uses another model |

What the field agrees on, and what Everruns does by default:

1. A routing agent beats self-organizing peers for business work.
2. Members do not get the conversation. They get the brief in `spawn_agent`.
3. Results come back as files or short reports, not transcripts.
4. One writer per file tree. Parallel writers get their own copy; git carries code.
5. Credentials and approvals stay with the person who started the run.
6. Runs with several agents cost more, so budgets must see the whole run.

## Model

- **Opt-in.** The Agents capability (`agent_handoff`) is the one switch, with no
  feature flag: anything opt-in through a capability needs none. An agent
  without it shows no Agents tab.
- **Members** are the capability's `targets` (alias `members`). Each entry needs
  only `agent_id`. On save the platform fills `id` (the agent's name, which the
  model uses as `spawn_agent` `target.id`), `name` and `description` from the
  member agent, so a rename or new description reaches every list at the next
  save. A member that is not an active agent of the organization is rejected.
- **Harness.** A member runs on its own current harness, looked up when the
  hand-off starts. `harness_id` remains an optional override.
- **Created here.** `agents.owner_agent_id` records the agent a member was made
  as part of. It only groups and exports: the member runs, is edited and costs
  like any agent. Removing it from the list never deletes it.
- **One level.** Hand-offs start only from the first session of a run (existing
  handoff rule), and a bundled agent cannot have members of its own.
- **Loops.** The Add agent sheet refuses an agent that already hands work to
  this one.

## What members share (defaults)

| Thing | Default | More, when needed |
|---|---|---|
| Conversation | Not shared; the brief only | n/a |
| Files | Own files | A copy of this session's files, set per member |
| Team folder | A `team/` folder every agent in the run reads and writes | Always on |
| Sandbox | Own, from its own harness | Not shared in v1: git branches carry code work |
| Memory, tools, model, harness | Its own | n/a |
| Connections and credentials | Resolved for the person; never passed | n/a |
| Spend | One pool per run (the existing root pool) | Per-hand-off cap (part 3) |

## Package format

Members live in the file system next to the agent, the same way skills do:

```
factory/
├── agent.toml            # members = ["everruns-review"]  (linked, by name)
├── instructions.md
└── .agents/agents/
    ├── implement/        # a full agent folder: agent.toml, instructions.md, files
    └── triage.md         # a one-file agent: front matter + instructions
```

- `.agents/agents/` is discovered in folders and archives. Bundled agents are
  never the parent's starting files. Text formats (JSON, TOML, YAML, Markdown)
  inline them as `agents = [...]`.
- `members = [...]` lists linked agents by name; they must exist at the
  destination. An explicit `members = []` turns discovery off.
- Platform export bundles created-here members and lists the rest by name.
  Import creates or updates the bundled agents (a name taken by an agent that
  was not created as part of this one is a conflict), resolves linked names,
  writes the Agents capability, and marks the bundled agents as created here.
  Everything is checked before anything is written.
- Hand-off settings that are not portable (`required_connections`,
  `required_scopes`, a harness override) are not exported.

Source: `crates/core/src/agent_package/` (codec), and
`crates/server/src/domains/agents/packages.rs` (platform binding).

## Approvals and questions

Any agent in a run can need an approval or ask a question. They are shown and
answered on the run's first session, the one the person talks to. The session
page stays a read-only debugging view and never asks.

- Requests are read, not copied: the first session lists the waiting sessions
  of its run (`root_session_id`) and reads each one's last
  `tool.call_requested`. Nothing is mirrored into the first session's events,
  where the runtime would read a copy as its own pending batch.
- `GET /v1/sessions/{id}/run-requests` lists them with the asking agent and
  session. Chat and Playground render the usual cards from it, headed with the
  agent's name; a card answers on the asking session.
- `POST …/tool-approvals` and `POST …/question-answers` on the first session
  route each answer to the session that waits on that call, then run that
  session's own validation. Answers naming two agents' requests are refused.
  Answering on the asking session directly keeps working; the first answer
  wins.
- The Agent Execution API includes the run's requests in `pending_approvals` /
  `pending_questions`, each with `session_id` and `agent`, and routes answers
  the same way, so Work Chat and other API callers get them.
- Only a first session sees or routes them, and an agent can never approve
  (`TM-AGENT-039`).

Source: `crates/server/src/domains/sessions/run_requests.rs`,
`crates/server/src/api/run_requests.rs`,
`apps/ui/src/components/chat/run-request-cards.tsx`. Slack delivery of another
agent's request is a follow-up.

## Team folder

Every agent in a run sees one `team/` folder in its files, mounted at `/team`.
What one agent writes there the others read at once, and it outlives every
session of the run. The Agents capability's prompt asks the routing agent to
pass paths instead of pasting long content and to have each agent write under
`team/<its name>/`.

- **Storage.** A Memory with scope `run`, owned by the run's first session
  (`owner_session_id`), created on first use. It reuses the memory mount's
  write-through, and organization Memory lists show only `org` scope, so a run
  folder never appears among the organization's memories.
- **Who sees it.** Every session started under another one, and a first
  session only when its agent has the Agents capability. Both are fixed for
  a session's life, so the per-process mount cache stays truthful.
- **Isolation.** One folder per run: a second run of the same agent gets its
  own. A session outside any run has no `/team`, and a write there stays in
  its own files.

Code: `crates/server/src/domains/session_files/memory_mounts.rs`
(`team_folder_owner`), `ensure_run_memory` in the memory repository,
migration `20261103050712_run_team_folder`.

## Copy of the files

A member set to **A copy of this agent's** (`files: "copy"` on its entry in
the Agents capability, the Files control on the Agents tab) starts each
hand-off with a copy of the handing-off session's files, the existing
`workspace` seed. It changes the copy without touching the originals and
hands results back through the team folder or a git branch. "The same files"
is not offered: two agents writing one set of files overwrite each other. An
explicit `seed` on `spawn_agent` still wins. The export keeps member names
only, so the setting is not carried in a package yet.

## Run conversation (part 2)

Agents in one run can message each other with `send_message`, mentions and
reactions as acknowledgements, under a message cap. It is kept separate so it
can move onto shared boards if that design is accepted.

## Costs (part 3)

Costs "By agent" stays as is. Usage records gain run attribution (the agent
that started the run), a "Runs that use other agents" card shows whole-run
cost, a usage policy can limit a whole run, and the routing agent can cap each
hand-off. Today an agent's usage policy does not cover its members' spend; only
the session-level root pool is shared.

## Traces (part 3)

A hand-off step shows its outcome, a member's session shows "Woken by", and the
run view lays out every agent's session as lanes on one timeline with a
breadcrumb back to the first session. The Cost tab can show the whole run.

## Framework

`AgentBuilder::member(agent)` adds a member as an ordinary `Agent` value; the
lead gets the Agents capability with one target per member, keyed by name
(`crates/everruns/src/agent/members.rs`). A member's id is derived from its
name, so it is the same on every build and restart. Member sessions run in the
lead's Engine on the lead's host:

- SQLite hosting: the member runs a build derived from the lead's
  (`<lead>:agent:<member id>`, `Engine::member_build`), resolved from the
  lead's registration, so registering the lead is enough after a restart.
- Local profile (legacy lane): the coordination runner attaches the member.

Packages: bundled agents are built with the lead's providers and host
services; linked names must be added with `.member()` or the build fails.
Export writes every member as bundled, since the Framework keeps no catalog
for a linked name. Invite mode is Platform-only (it reads stored agents).

## serve (follow-up)

serve gains `.member("name")` for other `#[agent]`s in the app and drops
`#[agent(sub)]` and its `ask_<name>` tools in favour of `spawn_agent`.
