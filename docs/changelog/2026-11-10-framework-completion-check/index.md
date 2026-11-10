---
title: Framework agents finish the work they announce
description: Embedded agents can turn on the completion check for every session and set their own continuation limits; working tasks are no longer cut at ten minutes.
date: 2026-11-10
kind: improvement
significance: minor
products: [framework, platform, cloud]
surfaces: [code]
links:
  docs: [/features/agent-triggers/#completion-check]
---

A Framework agent can now opt into the
[completion check](/features/agent-triggers/#completion-check) that trigger and
schedule sessions already get. With `CompletionCheck::Always`, a turn that ends
on "Fixing the test now" without the fix is reviewed on the agent's own model
and continues on its own, in the same turn:

```rust
use everruns::completion_check::{CompletionCheck, ContinuationLimits};

let agent = everruns::Agent::builder()
    .instructions("Fix the failing test.")
    .completion_check(CompletionCheck::Always)
    .continuation_limits(ContinuationLimits {
        max_turns: 5,
        max_tokens: 256_000,
        max_elapsed_secs: 600,
    })
    .build()?;
```

`continuation_limits` replaces the default caps for agents that spend more per
continuation, like a coding agent. The time limit now restarts whenever a
continuation runs tools, on the platform too. A long task that keeps working is
no longer stopped at ten minutes, while a stalled one still is.
