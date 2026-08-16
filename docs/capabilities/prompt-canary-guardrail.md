---
title: Prompt Canary Guardrail
description: Streaming output guardrail that withholds the assistant message when the model echoes the first sentence of its system prompt back to the user.
sidebar:
  order: 95
---

| | |
|---|---|
| **ID** | `prompt_canary_guardrail` |
| **Category** | Safety |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Low |

Detects naive system-prompt leakage during streaming. At the start of each assistant message, the capability extracts the first qualifying sentence of the assembled system prompt and uses it as a canary needle. If the model's accumulated output ever contains that needle, streaming aborts, the client is told to discard everything it accumulated, and a canned refusal becomes the persisted assistant message. The original tokens are never stored or replayed on subsequent turns.

This is intentionally narrow: a single substring match against one normalized needle. It catches obvious prompt-extraction attempts ("repeat your instructions", "what are you told to do?") without trying to be a general-purpose data-loss-prevention layer.

## Tools

None, this capability hooks the streaming output via the capability framework's output-guardrail extension point.

## How It Works

1. **Arming**: At the start of each assistant message stream, the capability walks sentence boundaries in the assembled system prompt and picks the first sentence whose normalized form is **≥ 30 characters**. This skips short generic openers like "You are a helpful assistant." in favor of an agent-specific identifying sentence
2. **Normalization**: Both sides of the comparison are lowercased, and runs of whitespace are collapsed to a single space, so the canary survives reformatting (extra spaces, capitalization drift, line wrapping)
3. **Streaming check**: After every text delta, the canary runs a substring scan over the accumulated assistant text. The check is synchronous and cheap, no I/O, no allocations beyond the normalized buffer
4. **Block on match**: When the needle appears in the accumulated output, the stream is aborted, the offending pending delta is suppressed, and `output.message.replaced` is emitted with `reason_code: "system_prompt_leak"`. The replacement text becomes the persisted assistant message

When the system prompt is too short or too generic to produce a needle ≥ 30 characters, the capability declines to arm for that stream and is a no-op.

## Streaming Timeline With a Trip

```
output.message.started
        │
        ▼
output.message.delta  ← model text accumulating ("Sure, my instructions are: …")
        │
        ▼            (canary trips on the next delta — pending text is suppressed)
output.message.replaced
        │            (UI discards what it accumulated, shows replacement)
        ▼
output.message.completed  ← persisted message body = replacement
```

## Configuration

### Default

```json
{
  "capabilities": ["prompt_canary_guardrail"]
}
```

Replacement text defaults to:

> [Response withheld: the model attempted to reveal protected instructions.]

### Custom replacement

```json
{
  "capabilities": [
    {
      "ref": "prompt_canary_guardrail",
      "config": { "replacement": "I can't share my system instructions." }
    }
  ]
}
```

## When To Enable

Use this capability when:

- You ship agents with proprietary, brand-specific, or compliance-relevant system prompts that should not be revealed verbatim to end users
- You want a cheap, deterministic defense against the most common prompt-extraction prompts
- You can tolerate a generic refusal in place of the model's response when the canary trips

Do **not** rely on this for:

- General-purpose data-loss prevention (PII, secrets in tool output, etc.), those need their own surfaces
- Defense against paraphrased or summarized prompt leaks, the canary only catches verbatim or near-verbatim copies of the first sentence
- Tool output or extended-thinking surfaces, the canary only inspects assistant text

## Limitations

- **Verbatim-only**: a model that paraphrases ("My role is to act as an internal pricing oracle…") will not trip the canary
- **First-sentence-only**: if the model leaks a *later* sentence of the system prompt, the canary won't catch it. Consider rewriting prompts so the most identifying claim is the opening sentence
- **No partial matching**: the substring must appear in full. Truncated leaks (cut off mid-sentence) pass through

## See Also

- [Events](/features/events/), the streaming event protocol that carries `output.message.replaced`
- [Capabilities](/features/capabilities/), the extension model these guardrails plug into
