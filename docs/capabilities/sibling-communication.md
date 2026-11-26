---
title: Sibling communication
description: Let the agents one run started message each other while they work.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `sibling_communication` |
| **Category** | Orchestration |
| **Dependencies** | None |

By default a spawned agent or a thread talks only to its parent. Add Sibling
communication to the parent and every agent it starts can message the others
in the same run:

- `message_agent` posts a message. Set `to` to an agent's name and that agent
  gets it in its own conversation; set `to` to `everyone` and every agent in
  the run does. Without `to`, the message is only posted for others to read.
- `read_run_conversation` reads the latest messages and lists the agents you
  can message.

The messages are kept under `shared/conversation/`, so the whole exchange is
readable in the first session's files. A message from another agent is always
marked as such and never counts as an answer to an approval or a question. A
run stops accepting messages after 200.

The retired id `run_conversation` is rewritten into `sibling_communication`.
