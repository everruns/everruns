---
title: How Memory Works
description: How Everruns agents remember across sessions: memory stores for the agent, team, parent, person and organization, typed Markdown entries and history.
---

An agent that forgets what was decided yesterday asks again, or worse, acts
against it. Everruns memory keeps decisions, rules, facts, preferences and
blocked items between sessions, in plain Markdown files the agent, a person and
any file tool can read. Nothing is hidden in a database the agent cannot see.

## Stores

A store is a folder under `/memory`. Each one answers "who shares this?":

| Store | Path | Shared by | Default |
|---|---|---|---|
| Agent | `/memory/agent` | Every session of one agent | read and write |
| Team | `/memory/team` | A coordinator agent and every thread or agent it starts | read and write |
| Parent | `/memory/parent` | The agent memory of the agent that started this one | read only |
| Person | `/memory/user` | One person's own sessions; nobody else | read and write |
| Organization | `/memory/org` | Every agent in the organization | read only |
| Named | `/memory/<name>` | The agents it is attached to | per attachment |

Every store is a switch in the [Memory capability](/capabilities/memory/), so a
coordinator can write to its team memory while a reviewer only reads it. The
agent's instructions describe each store it has, so it knows where a decision
belongs: a person's preference goes to their private memory, a team decision to
the team.

Agent, team and person memories are created the first time they are used.
Organization and named memories are created on the **Memory** page or with
`everruns memories create`.

## Entries

An entry is one Markdown file in a kind folder:

```text
/memory/team/decisions/everruns-5528.md
/memory/team/blocked/everruns-5528.md
/memory/agent/rules/merge-policy.md
/memory/user/preferences/reply-length.md
```

```markdown
---
kind: decision
subject: everruns#5528
occurred_at: 2026-11-26T04:10:00Z
recorded_by: agent:coordinator
source: slack
source_url: https://example.slack.com/archives/C1/p1764130200
---
Allowlist @a2ui/web_core instead of denying the upgrade.

> allow it, it's ours
```

| Kind | Use it for |
|---|---|
| `decision` | Something a person decided, which the agent must follow |
| `rule` | A standing instruction: "always squash merge" |
| `fact` | Something true about the world the agent works in |
| `preference` | How a person likes things done |
| `blocked` | Work waiting on someone, until it is resolved |
| `note` | Anything else worth keeping |

One subject holds one current entry of each kind. When the coordinator
remembers a new decision about `everruns#5528`, it replaces the old one, and
the old version stays in the store's history. The quote keeps the person's
exact words, so the agent can tell what was said from what it inferred.

Free-form files are welcome too: a runbook at `/memory/team/runbook.md` is part
of memory and `recall` searches it.

## The memory card

At the start of a session, and when memory changes, the agent gets a short card:
which stores it has, the open blocked items, and the latest decisions, rules
and preferences in each. The card is sent as a context update, not inside the
instructions, so it does not break prompt caching. Turn it off with
`"card": false`.

## Tools

The agent uses `remember` to record an entry, `recall` to search by words
(identifiers such as `everruns#5528` match exactly), `resolve` to close a
blocked item, `forget` to delete an entry, and `accept` to take a proposal. Read-only stores refuse writes
with an error the agent can act on.

## History

Every write and delete to a memory file is kept, with who made it. Forgetting
deletes the file, never its history. See a store's history on the **Memory**
page, or:

```bash
everruns memories history mem_01h9... --path /decisions/everruns-5528.md
```

## Proposals and dreaming

An entry can be recorded as a proposal instead: it waits under `/proposals/`,
off the card, until an agent with write access calls `accept`. Proposals are
meant for stores many agents share, where a consolidation pass should not
change what everyone follows on its own.

[Dreaming](/features/dreaming/) consolidates finished sessions into notes in
the person's memory, under `/memory/user/learned`.

## Seeing memory

The **Memory** page under **Blocks** lists every store you can see with its
entry count, open blocked items and proposals to review. A store's page groups
entries by kind, shows entry files that do not parse so they can be fixed, and
lists every change. From the command line:

```bash
everruns memories stores
everruns memories entries mem_01h9...
```

## Privacy

A person's memory is private to that person. Only their own sessions mount
`/memory/user`, and the Memory page and API show it only to them, whatever
their role in the organization. Never put personal context in agent, team or
organization memory, which other people's sessions read.

A session with no person (a scheduled or webhook run) has no person memory.

## See also

- [Memory capability](/capabilities/memory/), configuration and tools.
- [Session participants](/features/session-participants/), host and guest agent behavior in shared sessions.
- [Rooms](/features/rooms/) keep their own facts next to their messages.
