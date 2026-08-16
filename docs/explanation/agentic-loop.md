---
title: The agentic loop
description: Why agent execution is a bounded reason–act cycle, what execution phases mean, and how multi-step tool flows stay coherent across turns.
sidebar:
  order: 2
---

An "agent" is a loop, not a function call. This page explains the loop Everruns runs and why it's shaped the way it is.

## Reason, then act

Each **turn** is one iteration of:

1. **Reason.** Send the full conversation history (system prompt + messages + previous tool results) to the LLM. The model either produces text or requests tool calls.
2. **Act.** If the model requested tool calls, execute them in parallel. The results become new messages in the conversation.
3. **Loop.** If there were tool calls, go back to step 1. If the model produced a final text response, the turn completes.

A turn is capped at **10 iterations** by default. That cap exists for one reason: a misbehaving prompt can drive the model into an infinite tool-calling spiral, and an unbounded loop costs real money in tokens. The cap is configurable per session.

## Why parallel act?

The model often emits several tool calls in one response, "read these three files" or "fetch these two URLs". Executing them sequentially would serialize independent work for no reason. Everruns runs all tool calls from a single reason step concurrently and only re-enters reason once every call has completed (or failed).

This means tool implementations cannot assume ordering between calls within a single act phase. If two tools must run in order, the model has to emit them across two turns.

## Execution phases

When an assistant message includes tool calls, the model hasn't given a final answer yet, it's just narrating its plan. When the message has no tool calls, that's the final answer.

Everruns labels each assistant message with an **execution phase**: `Commentary` (intermediate, before/between tool calls) or `FinalAnswer` (completed response). Two consumers care:

- **The model.** Some providers (OpenAI Responses API on the GPT-5.4 and GPT-5.5 families) accept and return phase annotations on replayed history. Without them, models can mistake earlier commentary for completed answers and stop early on long flows.
- **The UI.** Phase tells the chat surface whether to keep the "thinking" indicator on or render the message as a final response.

Phases are derived from message state (presence of tool calls) and stored on the message. Providers that don't accept phases on the wire still get accurate internal tracking.

## Turn lifecycle

Every turn emits these events in order:

```
turn.started
  reason.started → reason.completed (one LLM call)
  act.started    → act.completed   (zero or more tool calls)
  ... (repeat reason/act until the model produces a final answer or max iterations) ...
turn.completed | turn.failed | turn.cancelled
```

Streaming text and tool calls produce `output.message.delta` and `tool.started` / `tool.completed` events in between. The full event catalog is in the [Event Reference](/event-reference/).

## Why turns are durable, not in-memory

A naïve implementation of the loop would hold all turn state in worker memory. Everruns doesn't, because a worker crash mid-turn would lose work the user already paid for.

Instead each step (reason, each tool call) is a separate durable task. The worker persists state after every step. If a worker crashes between steps, the control plane detects the missed heartbeat and re-queues the next task on a different worker. From the application's perspective: a brief delay, then the stream continues. No retry button required.

This trade-off, paying for a database write on every step, is what makes Everruns a *durable* agentic harness rather than a thin LLM wrapper. See [Durable execution](/explanation/durable-execution/).

## What happens when the loop "gets stuck"

Three failure modes show up in practice:

- **Runaway tool calls.** The model keeps calling tools without converging. Mitigated by the iteration cap; the turn fails with `turn.failed` once the cap is hit.
- **A tool that hangs.** Tool calls have configurable timeouts. The act phase reports the failure as a tool result so the model can recover on the next reason step.
- **The model rejects the prompt as too large.** Context [compaction](/advanced/compaction/) runs reactively and the request is retried. The conversation continues with older messages compressed.

In all three cases the session stays usable. You don't lose the conversation; you lose at most one turn.
