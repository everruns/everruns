---
title: Build a foreman agent
description: Put one agent in front of a team of specialists, so a Slack mention is triaged and delegated to the right worker instead of answered by a generalist.
appliesTo: [platform, cloud]
---

A **foreman** is an agent whose job is routing, not answering. It receives an
ambiguous request, decides which specialist should handle it, delegates, and
reports back. The specialists are ordinary Agents with their own prompts, tools,
and models.

This is server-side delegation. If you want the *application* to chain agents in
sequence, see [Orchestrate multi-agent
pipelines](/how-to/orchestrate-multi-agent-pipelines/) instead — that pattern
keeps control in your code. A foreman keeps control in the agent, which is what
you want when the trigger is a human asking for something in Slack.

## Prerequisites

- Two or more Agents to delegate to, each already working on its own.
- An agent to act as the foreman.
- For the Slack front door: a Slack workspace where you can create apps.

## Step 1 — Get the workers right first

Build and test each specialist on its own before wiring any delegation. A
foreman that routes correctly to a broken worker looks like a broken foreman,
and you will debug the wrong layer.

Give each one a narrow prompt and only the capabilities it needs. "Reviews Rust
diffs for correctness bugs" routes better than "helps with code", because the
foreman picks targets from their descriptions.

Note each worker's agent id (`agent_...`). Its name and description are what
the foreman sees when it picks one.

## Step 2 — Give the foreman Threads and Coordinator

A foreman follows the work it hands out: it hears when a worker finishes or
asks something, sends follow-ups, and closes the work when the requester is
happy. That is [Threads](/capabilities/threads/). Add
[Coordinator](/capabilities/coordinator/) next to it, so the foreman routes
instead of doing the work itself:

```json
[
  {
    "ref": "threads",
    "config": {
      "self": false,
      "agents": [
        { "agent_id": "agent_...code_reviewer", "required_connections": ["github"] },
        { "agent_id": "agent_...incident_responder" }
      ],
      "max_open": 6
    }
  },
  { "ref": "coordinator" }
]
```

`agents` is an explicit allowlist; the platform fills each entry's name and
description from the agent when it saves. In the UI, add workers from the
foreman's **Team** tab. `"self": false` keeps the foreman from working
threads itself; leave it on if it should also take generalist work.

Two properties shape what the foreman can do:

- The foreman **does not inherit a worker's tools** and never receives its
  provider credentials. It can ask a worker to act; it cannot act as the
  worker. Each worker runs as itself, resolved for the person who asked.
- `required_connections` refuses a thread up front when a provider connection
  is missing, instead of failing mid-run.

Do not also give the foreman Spawn agents: an agent has one or the other.

## Step 3 — Write the foreman's prompt

Coordinator already tells the foreman to put real work in threads, one thread
per piece of work, follow-ups to the thread that owns it, and to relay results.
Your prompt adds what only you know:

1. **When to delegate and when to answer.** Be concrete: "If the request names
   a file, a diff, or a PR, start a thread on code_reviewer."
2. **How to choose between overlapping workers**, and what to do when none
   fits: usually ask a clarifying question rather than guessing.
3. **How to report back**: one line in the Slack thread when work starts and
   when it finishes. A foreman that delegates silently is worse than none.

Keep it short. A long routing prompt tends to produce a foreman that reasons
about routing out loud in your Slack channel.

## Step 4 — Understand the thread tools

| Tool | Why a foreman cares |
|---|---|
| `start_thread` | `title`, a complete `brief` (the worker sees nothing else), and `agent`: the worker's name. Returns a `thread_id` at once, so the foreman stays responsive. **The brief must not contain credentials.** |
| `message_thread` | Relays a follow-up from Slack to the thread that owns the work, even after it finished. |
| `list_threads`, `get_thread` | Where each piece of work stands, and the workers available. |
| `close_thread` | Closes the work once the requester accepts it; `cancel` stops it. |

A worker keeps a checklist, asks with `ask_parent`, and ends with `finish`.
Each of those wakes the foreman with an automatic update; it does not poll,
and you should not prompt it to. Workers write long output to `shared/`,
which the foreman reads.

### Threads or spawned agents?

| | Threads | [Spawn agents](/capabilities/spawn-agents/) |
|---|---|---|
| Shape | Followed work the foreman steers and closes | One call: brief in, result out |
| Use when | The requester will come back, steer, or accept the work | Quick parallel checks inside one answer |

A foreman in front of people wants Threads. A worker that fans out "review
these six files" inside its own thread wants Spawn agents on the worker.

## Step 5 — Put it in front of Slack

Expose the **foreman** — and only the foreman — on Slack. The workers stay
internal; they are reached through delegation, not by being mentioned.

Follow [Publish an agent as a Slack app](/how-to/publish-to-slack/) for the
mechanics. Two choices matter for a foreman:

- **`session_strategy: per_thread`** (new channels default to `per_channel`, so set
  it explicitly). Each Slack thread becomes one
  foreman session, which is what you want: the thread is the unit of work, and
  the foreman keeps its delegation state for the life of that thread.
- **Enable the agent surface** (`agent_surface_enabled`) if you want the foreman
  available in Slack's assistant pane as well as in channels. The pane streams
  replies token-by-token and shows a status line while tools run.

## Step 6 — Let the foreman act on Slack (optional)

Out of the box an agent can reply in its own thread and nothing else. It cannot
add a reaction, send a DM, look someone up, or post to another channel.

If your foreman needs those — acknowledging a request with an emoji while work
runs is the common one — attach a Slack MCP server as a capability on the
foreman. MCP servers appear as virtual capabilities alongside built-in ones.

Be aware this means a second Slack token, separate from the channel's bot token,
with its own scopes to manage and rotate.

## Step 7 — Test the routing, not the workers

Send the foreman requests that are deliberately near the boundary between two
targets, and requests that match none. Those are where routing fails. A request
that obviously belongs to one worker will pass whether or not your prompt is any
good.

Check that a dispatched request reports back into the thread that asked. A
foreman that accepts work and reports somewhere else trains people to stop using
it.

## Limits worth knowing before you commit

- **No approval buttons.** Slack interactivity is not wired up, so a foreman
  cannot ask "approve this?" with a button and act on the click. It can only ask
  in prose and read the reply. For a foreman that dispatches consequential work,
  this is the real constraint.
- **Progress is per-turn, not per-thread.** The Slack status line reflects the
  foreman's current turn. "3 of 5 threads finished" is available to the foreman
  via `list_threads` but is not rendered into Slack for you; the foreman has to
  say it.
- **Two identities** if you use a Slack MCP server, as above.

## See also

- [Threads](/capabilities/threads/) and [Coordinator](/capabilities/coordinator/)
- [Spawn agents](/capabilities/spawn-agents/) — one-off calls
- [Publish an agent as a Slack app](/how-to/publish-to-slack/)
- [Orchestrate multi-agent pipelines](/how-to/orchestrate-multi-agent-pipelines/) — the client-side alternative
