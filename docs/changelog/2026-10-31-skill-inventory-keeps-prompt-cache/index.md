---
title: Skill installation preserves cached prompt prefixes
description: Installing or editing skills appends a recorded inventory update instead of rewriting system instructions, preserving earlier prompt prefixes.
date: 2026-10-31
kind: fix
significance: notable
products: [platform, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/agent-skills/]
---

Installing, editing, or removing skills now appends an inventory update to the
conversation instead of rewriting the system prompt. Earlier updates replay
at their original positions after a worker restart, and an unchanged
inventory adds no repeated update.

Workspace and multi-scope skills retain their discovery and activation tools,
including optional inspection reports. Removed inventories receive an explicit
clearing update. Sessions carrying these updates use local history reduction.

In the Framework, enable `everruns-core`'s `builtins` feature. Discovery
instructions and the current inventory have separate contributions:

```rust
use everruns_core::capabilities::{Capability, SystemPromptContext};
use everruns_core::builtins::SkillsCapability;

async fn inspect_discovery(ctx: &SystemPromptContext) {
    let instructions = SkillsCapability.system_prompt_contribution(ctx).await;
    let inventory = SkillsCapability
        .conversation_context_update_contribution(ctx)
        .await;
    assert!(instructions.is_some());
    // The runtime records inventory changes at the conversation tail.
    let _ = inventory;
}
```

See [Agent Skills](/capabilities/agent-skills/) for installation and activation.
