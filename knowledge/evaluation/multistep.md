---
type: Specification
title: "Multistep eval cases"
description: "Simulated users that carry a conversation from a persona and goal, and tool fixtures with canned responses."
tags:
  - everruns
  - evaluation
---
# Multistep eval cases

## Purpose

A single exchange shows how an agent answers one message. Most agent work is
a conversation: the user clarifies, pushes back, adds details, and the agent
calls tools along the way. A multistep case tests that conversation without a
person in the loop and without touching real systems.

This is step 5 of the Evals rebuild. The case page with its Scripted and
Simulated modes follows approved mockup 05.

## Simulated user

A case can carry a simulated user: a persona, a goal, an optional "done when"
condition, a turn limit (default 8, at most 30), an optional model and an
optional virtual user.

- The simulator continues where the scripted messages stop. A case can open
  with fixed messages and let the simulator carry on, or have no scripted
  messages at all.
- After each settled agent turn the simulator sees the conversation so far
  and writes the user's next message, or says it is done. It ends when it
  says so or at its turn limit. The result records which, how many messages
  it sent and the model that played it, because a score is a statement about
  the agent and simulator pair.
- The simulator is a plain completion on the org's own models, resolved and
  billed like the LLM judge. Its spend counts toward the run's cap as each
  message is written, so a long simulation stops when the money does.
- With a virtual user, the session runs as that end user, so the agent sees
  their identity, connections and memory scope, as Playground's "send as"
  does.
- Verification (`post`) messages do not combine with a simulator: a resumed
  case could not tell which phase it stopped in.
- A deployment with no model stack refuses simulated cases with the reason
  before it creates a session.

Judges and decision graders of a multistep case see the whole conversation,
not only the last message.

## Tool fixtures

A fixture is a tool the case gives the agent, with a canned response. It is
declared to the agent as a client-side tool; the runner answers every call to
it while it waits for the turn, and the call stays in the trace. That lets a
case check what the agent sent ("`create_ticket` called with ORD-48213") with
the `tool_called` grader's `arguments_contain`, without touching a real
system.

- A client-side call parks a turn only when the session says its client
  answers such calls (the `setup_connection` hint). Eval sessions with
  fixtures set it, or the turn would go on without the canned response.
- A fixture adds a tool. It does not intercept a server tool of the same
  name.
- A call to a client-side tool with no fixture, one the agent declares
  itself, is answered with an error so the case goes on instead of waiting
  out its timeout.
- Limits: 20 fixtures per case, names of letters, digits, `_` and `-`, a
  64 KB response and a 16 KB parameter schema.

## Where it lives

- Case fields and validation: `crates/server/src/domains/evals/multistep.rs`.
- The conversation loop and fixture answers:
  `crates/server/src/domains/evals/runner.rs`.
- The simulator's prompt and parsing: `multistep.rs`, on the shared
  completion in `crates/server/src/domains/observers/judge.rs`.

Related: [Evals](evals.md), [Eval experiments](experiments.md).
