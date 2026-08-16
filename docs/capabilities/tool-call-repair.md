---
title: Tool Call Repair
description: Detects and repairs malformed tool-call arguments from the model, recovering the turn instead of surfacing a raw parse error.
sidebar:
  order: 96
---

| | |
|---|---|
| **ID** | `tool_call_repair` |
| **Category** | Safety |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Low |

Opt-in recovery net for malformed tool calls. Models occasionally emit
tool-call arguments that are not clean JSON, wrapped in a Markdown code
fence, surrounded by prose, with trailing commas or single quotes, or with values
typed as strings where the schema wants numbers. Without repair, such a call
either surfaces a parse error or silently collapses to empty arguments and
fails downstream. This capability salvages the call so the turn proceeds.

**Disabled by default.** The capability is registered so agents can enable it,
but contributes nothing unless explicitly selected. With it off, behavior is
byte-for-byte unchanged.

## Tools

None, the capability intercepts inside the `reason` step, after the model's
tool calls are finalized and before the assistant message is built.

## How It Works

1. **Deterministic local salvage**: A pure function runs over each call's
   `arguments`: it unwraps fenced code blocks and strips surrounding prose,
   removes trailing commas, normalizes single quotes to double quotes, and
   coerces string-typed known keys to the type declared by the tool's JSON
   schema (e.g. `"42"` → `42` for an integer property). An already-valid call
   is a no-op.
2. **Bounded corrective re-prompt**: When local salvage cannot recover a usable
   object, the capability allows up to `max_reprompts` attempts per call (default
   1) before falling through to the normal error path. The re-prompt is realized
   by the agent loop: the unrepaired call proceeds to today's error path and the
   model retries on the next iteration. The per-call cap guarantees there is no
   infinite repair loop.
3. **Observability**: Each malformed call emits one `tool.call_repaired` event
   carrying an outcome label: `local-salvage`, `re-prompt`, or `gave-up`.

## Configuration

### Default

```json
{
  "capabilities": ["tool_call_repair"]
}
```

### Custom re-prompt cap

```json
{
  "capabilities": [
    {
      "ref": "tool_call_repair",
      "config": { "max_reprompts": 2 }
    }
  ]
}
```

`max_reprompts` accepts `0`–`5`. `0` means "salvage locally or fall straight
through to the error path with no re-prompt".

## When To Enable

Use this capability when:

- You run models or providers that occasionally wrap tool arguments in prose or
  code fences, or emit lenient JSON (single quotes, trailing commas)
- You want a malformed call to recover the turn rather than waste an iteration on
  a raw parse error

## Limitations

- **Verbatim JSON only**: salvage extracts an embedded JSON object; it does not
  invent missing required fields or guess intent from natural language
- **Bounded input**: argument blobs larger than 256 KiB are treated as
  un-salvageable without parsing (a denial-of-service guard against runaway model
  output)
- **No deep type checking**: coercion handles top-level `integer` / `number` /
  `boolean` string values; full schema validation remains the tool's job

## See Also

- [Events](/features/events/), the streaming event protocol that carries `tool.call_repaired`
- [Capabilities](/features/capabilities/), the extension model this capability plugs into
