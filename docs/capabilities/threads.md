---
title: Threads
description: Hand off followed work to threads. The agent starts a thread, sends it follow-ups, hears when it finishes or asks, and closes it when the work is accepted.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `threads` |
| **Category** | Orchestration |
| **Features** | `threads` |
| **Dependencies** | None |

Threads lets an agent hand off **followed work**. A thread is a separate
session working one piece of work. The agent starts it with a brief, sends it
follow-ups, is told when it finishes, asks something or stops, and closes it
when the person accepts the work. The person keeps talking to one agent while
the work happens elsewhere, and can open any thread to watch it.

Use Threads for work the person will come back to. For one-off calls, use
[Spawn agents](/capabilities/spawn-agents/). An agent has one or the other,
never both: Threads already cover one-off work. Add
[Coordinator](/capabilities/coordinator/) to make the agent route work to
threads instead of doing it itself.

## Configuration

```json
{
  "ref": "threads",
  "config": {
    "self": true,
    "agents": [{ "agent_id": "agent_..." }],
    "max_open": 4
  }
}
```

| Field | Default | Description |
|---|---|---|
| `self` | `true` | A thread may run this agent |
| `agents` | none | Other agents a thread may run, each as itself |
| `any_agent` | `false` | Any agent the person may run, by agent id |
| `max_open` | 8 | Threads working at once (1 to 64) |
| `max_total` | 100 | Threads one session may start (1 to 1000) |

## Tools

| Tool | What it does |
|---|---|
| `start_thread` | Start a thread with a `title`, a complete `brief`, and optionally `agent` (`self` or an agent's name). Returns the `thread_id` |
| `message_thread` | Send a follow-up. While the thread works, it joins the current work; when it is idle, it becomes new work |
| `list_threads` | The threads and where each stands, plus the agents a thread may run |
| `get_thread` | One thread: its checklist, summary, question and latest messages |
| `close_thread` | Close a thread when the work is accepted. `cancel` stops work in progress; `reopen` opens a closed thread again |

A thread reports through tools the host gives it:

| Tool | What it does |
|---|---|
| `report_progress` | Update its checklist, or tell the parent something now |
| `ask_parent` | Ask a question the brief leaves open, with options |
| `finish` | Finish with a summary, how it checked the work, and any artifacts |
| `hand_back` | Hand back a request that belongs elsewhere |

Every report wakes the parent with an automatic update. Threads do not start
threads of their own.

## Shared folder

Every thread in one run shares the `shared/` folder. Pass file paths in the
brief instead of pasting long content.

## Where threads can start

Threads start from the console, from a Work Chat, from a Slack conversation
and from runs no person sent (a schedule, a webhook or a trigger, including a
trigger aimed at a Slack conversation). Outside the console the agent can only
run the agents its Threads settings name, and threads stay out of the channel:
only the agent itself answers in Slack. A Slack member's follow-ups to a thread
are still attributed to that member.

## Notes

- The retired id `coordination` is rewritten into `threads` plus
  `coordinator` when an agent is read or saved.
- Threads talk to each other only with
  [Sibling communication](/capabilities/sibling-communication/).

## See also

- [Coordinator](/capabilities/coordinator/)
- [Spawn agents](/capabilities/spawn-agents/)
- [Framework: agents that start agents](/framework/coordination/)
