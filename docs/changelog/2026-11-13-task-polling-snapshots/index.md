---
title: Shorter responses for unchanged task polls
description: Reuse a task list snapshot to avoid receiving unchanged background task lists while preserving progress and input requests.
date: 2026-11-13
kind: improvement
significance: minor
products: [framework, platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/sub-agents/]
---

The `list_tasks` tool returns a snapshot that agents can pass as `since` on the
next poll with the same filters. When nothing changed, it returns a short
`unchanged` response instead of repeating every background task. Progress,
state details, pending input requests, and outcomes still produce a fresh list.
Heartbeat timestamps alone do not.

Omit `since` to retrieve the list again, or use `get_task` for full details.
Snapshots work across turns without relying on a particular worker process.

For a Rust host with `everruns-capabilities` and `everruns-core` (no additional
features required), reuse the tool response in the same session context:

```rust
use everruns_capabilities::capabilities::session_tasks::ListTasksTool;
use everruns_core::{tool_context::ToolContext, tools::{Tool, ToolExecutionResult}};
use serde_json::json;

async fn poll(context: &ToolContext) -> ToolExecutionResult {
    let first = ListTasksTool.execute_with_context(json!({}), context).await;
    let ToolExecutionResult::Success(value) = first else { return first };
    ListTasksTool
        .execute_with_context(json!({"since": value["snapshot"]}), context)
        .await
}
```

See [sub-agents](/capabilities/sub-agents/) for background task controls.
