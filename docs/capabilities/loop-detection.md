---
title: Tool Loop Detection
description: Detects an agent repeating the same tool calls, results, failed edits, or file reads, and injects a warning that tells it to change approach.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `loop_detection` |
| **Tools** | None |
| **Features** | None |
| **Dependencies** | None |

Tool loop detection watches the conversation for an agent that keeps doing the
same thing without progress. When it finds a loop, it adds a short system
message after the latest input telling the model what it is repeating and what
to do instead: change the arguments, look at a different source, or report the
blocker.

It does not stop the turn, block a call, or change any tool result. The warning
is rebuilt from the conversation each time messages are loaded for the model,
so it is never stored in session history, and later requests replay the same
prompt prefix.

The [Generic](/built-ins/harnesses/generic/) and
[Platform Chat](/built-ins/harnesses/platform-chat/) harnesses include it.

## What counts as a loop

The checks run in this order, and the first match produces the warning:

| Pattern | Triggers at |
|---|---|
| A mutating tool (`edit_file`, `write_file`, `delete_file`, `bash`, or an MCP tool ending in `__` plus one of those names) failing the same way with identical arguments, in consecutive calls | `mutating_failure_threshold`, default 2 |
| The same tool call returning the same result, in consecutive calls since the last user message | `threshold`, default 3 |
| Repeated reads of the same file (`read_file`, `read_many_files`) at the same offset, mixed with reads of other ranges of that file | `threshold`, default 3 |
| The same batch of tool calls with identical arguments in consecutive assistant messages, regardless of call order inside the batch | `threshold`, default 3 |

The lower threshold for failed mutations interrupts a broken edit or command
before a third identical attempt with side effects.

## Configuration

```json
{
  "ref": "loop_detection",
  "config": { "threshold": 3, "mutating_failure_threshold": 2 }
}
```

| Field | Default | Description |
|---|---|---|
| `threshold` | `3` | Repetitions of the same call batch, call result, or read range that count as a loop |
| `mutating_failure_threshold` | `2` | Identical failed results from a mutating tool that count as a loop |

Both must be integers of at least 1.

In the [Framework](/framework/advanced-capabilities/), add it by ID:

```rust ignore
use everruns::CapabilityRef;
use serde_json::json;

let agent = Agent::builder()
    // ...
    .capability(CapabilityRef::new("loop_detection").config(json!({ "threshold": 4 })))
    .build()?;
```

## See also

- [Tool Call Repair](/capabilities/tool-call-repair/), recovery for malformed tool-call arguments
- [Generic harness](/built-ins/harnesses/generic/), the default capability set that includes it
