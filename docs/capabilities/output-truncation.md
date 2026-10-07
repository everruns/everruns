---
title: Output Truncation
description: Chooses what a turn does when a model response hits the output token limit in the middle of a tool call, retry with a notice, fail, or carry on.
sidebar:
  order: 96
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `output_truncation` |
| **Category** | Safety |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Low |

A model response that reaches its output token limit (finish reason `length`)
can stop in the middle of a tool call. Everruns never runs such a call, and
never runs it with empty (`{}`) arguments in place of the ones the model did
not finish. This capability chooses what the turn does next. On OpenAI
Responses and Bedrock it also applies when a finished call arrives with
arguments that are not valid JSON.

Every agent gets the default policy, `continue`, without adding the capability.
Add it only to change the policy or the retry limit.

## Tools

None, this capability only configures the turn loop.

## How It Works

When a response loses tool calls this way:

- Calls in the same response whose arguments arrived complete still run.
- The cut-off calls do not run. Every provider drops them: Anthropic, OpenAI
  (Chat Completions and Responses), Gemini, and Bedrock.
- The policy decides the turn's next step:

| Policy | What happens |
|---|---|
| `continue` (default) | The model gets a short notice that its output hit the limit and the call was not run, and the turn runs another model call so it can retry with less output. After `max_retries` cut-off responses in a row, the turn fails with an error. |
| `fail` | The turn ends with an error. |
| `off` | The turn carries on as if the model had not asked for the cut-off calls: it ends on the partial answer, with stop reason `max_tokens`. |

The `continue` notice appears in the conversation the model sees, right after
the cut-off response and any results of the calls that did run. It is not
stored as a message and is not shown in the chat. A clean model response, or
a new user message, resets the count of consecutive cut-off responses.

Responses that are refused or filtered (`content_filter`, `refusal`) are not
retried: retrying does not fix them.

## Config

```json
{
  "capabilities": [
    {
      "ref": "output_truncation",
      "config": { "policy": "continue", "max_retries": 2 }
    }
  ]
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `policy` | `continue` \| `fail` \| `off` | `continue` | What the turn does when a response loses tool calls |
| `max_retries` | integer, 0 to 10 | `2` | Cut-off responses in a row `continue` retries before the turn fails |

When the capability is set on both the harness and the agent, the agent's
config wins.

## Observability

Each `llm.generation` event records how the response ended:

- `finish_reasons`;
- `tool_calls_dropped`;
- `truncation_gate`, set to `retried` or `failed` when the policy acted.

The same value is exported as the `everruns.llm.truncation_gate` span
attribute and counted in the `everruns_llm_truncation_gate_total` metric. See
the [event reference](/event-reference/#llmgeneration).

## Related

- [Tool Call Repair](/capabilities/tool-call-repair/), recovery for tool-call
  arguments that are almost valid JSON
- [Tool Loop Detection](/capabilities/loop-detection/)
