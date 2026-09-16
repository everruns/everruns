---
title: Build a foreman agent
description: Put one agent in front of a team of specialists, so a Slack mention is triaged and delegated to the right worker instead of answered by a generalist.
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

Note their agent ids (`agent_...`).

## Step 2 — Give the foreman the `agent_handoff` capability

Delegation targets are an explicit allowlist on the foreman. Configure the
`agent_handoff` capability with one entry per worker:

```json
{
  "targets": [
    {
      "id": "code_reviewer",
      "name": "Code Reviewer",
      "description": "Reviews diffs and pull requests for correctness bugs",
      "agent_id": "agent_...",
      "required_connections": [],
      "required_scopes": []
    },
    {
      "id": "incident_responder",
      "name": "Incident Responder",
      "description": "Investigates alerts and production incidents",
      "agent_id": "agent_..."
    }
  ]
}
```

`description` is the field the model reads when choosing a target, so write it
for that purpose. `id` is the stable key the foreman passes back in tool calls;
keep it short.

Two properties worth knowing, because they shape what the foreman can do:

- The foreman **does not inherit the target's tools**, and never receives the
  target's provider credentials. It can ask a worker to act; it cannot act as
  the worker.
- `required_connections` gates a handoff on a provider connection existing
  before the run starts, which turns a mid-run credential failure into an
  up-front refusal.

`required_scopes` are audit labels in the current implementation, not enforced
grants — a tool that needs hard authorization checks its own scopes before
acting.

## Step 3 — Write the foreman's prompt

This is the part that decides whether the thing works, and the part no
configuration can do for you. A foreman's prompt needs four things:

1. **When to delegate and when to answer.** Without this, a foreman either
   delegates trivia or answers things it should have routed. Be concrete: "If
   the request names a file, a diff, or a PR, hand off to `code_reviewer`."
2. **How to choose between overlapping targets**, and what to do when none fits
   — usually ask a clarifying question rather than guessing.
3. **What to do while work is running.** Background handoffs return
   immediately; the foreman must know to report that it has dispatched, not to
   invent a result.
4. **How to report back.** A foreman that delegates silently is worse than no
   foreman, because the requester cannot tell whether anything is happening.

Keep it short. A long routing prompt tends to produce a foreman that reasons
about routing out loud in your Slack channel.

## Step 4 — Understand what `spawn_agent` gives you

The foreman delegates with `spawn_agent`, using `target.type = "agent"` and the
`target.id` from your config. The parameters that matter for a foreman:

| Parameter | Why a foreman cares |
|---|---|
| `mode` | `background` (default) returns a `task_id` immediately so the foreman can dispatch several workers and stay responsive. `foreground` blocks until the child finishes. `invite` joins the target into the *current* session instead of a child one. |
| `result_schema` | A JSON Schema the child must satisfy. Turns "whatever the worker said" into a structured result the foreman can act on rather than re-parse. |
| `public_context` | Non-secret context appended to the child task. Use it for the Slack thread reference so the worker knows where the request came from. |
| `instructions` | The work request. **Must not contain credentials** — the child has its own. |

Background handoffs create a task with `wake_policy = on_terminal`, so the
foreman is woken when a worker finishes. It does not poll, and you should not
prompt it to.

While work is in flight the foreman manages it with the generic task tools:
`list_tasks`, `get_task`, `message_task` to steer a worker mid-run, and
`cancel_task`. These work identically for subagents, so a foreman can mix both
kinds of delegation.

### Subagent or handoff?

Both go through `spawn_agent`; the difference is what the child *is*.

| | `target.type = "subagent"` | `target.type = "agent"` |
|---|---|---|
| Child config | Inherits the foreman's harness and agent configuration | The target Agent's own prompt, capabilities, MCP servers, model |
| Set-up cost | None — spawn by name | Build and allowlist the Agent first |
| Use when | The work is the same kind of work, just parallel | The work needs different tools, a different model, or different authority |

A foreman that fans out "review these six files" wants subagents. A foreman that
routes "is this a code question or an incident?" wants handoff. Most real ones
use both.

Subagent fan-out needs the separate `subagents` capability on the foreman — see
[Sub Agents](/capabilities/sub-agents/). Nesting is depth-governed
(`max_subagent_depth`) with root-tree caps on live and total descendant tasks, so
a foreman cannot fork-bomb your org by accident.

## Step 5 — Put it in front of Slack

Expose the **foreman** — and only the foreman — on Slack. The workers stay
internal; they are reached through delegation, not by being mentioned.

Follow [Publish an agent as a Slack app](/how-to/publish-to-slack/) for the
mechanics. Two choices matter for a foreman:

- **`session_strategy: per_thread`** (the default). Each Slack thread becomes one
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
- **Progress is per-turn, not per-task.** The Slack status line reflects the
  foreman's current turn. "3 of 5 workers finished" is available to the foreman
  via `list_tasks` but is not rendered into Slack for you; the foreman has to
  say it.
- **Two identities** if you use a Slack MCP server, as above.

## See also

- [Sub Agents](/capabilities/sub-agents/) — the `subagents` capability and the shared `spawn_agent` dispatcher
- [Publish an agent as a Slack app](/how-to/publish-to-slack/)
- [Orchestrate multi-agent pipelines](/how-to/orchestrate-multi-agent-pipelines/) — the client-side alternative
