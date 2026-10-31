---
title: Exact file edit failures show recovery hints
description: Failed exact file edits now show bounded similar-line hints and cautious already-applied guidance, helping agents recover without changing files.
date: 2026-10-31
kind: improvement
significance: minor
products: [framework, platform, cloud]
surfaces: [agent, code]
links:
  docs: [/capabilities/file-system/]
---

Failed `edit_file` calls now identify the failed replacement and show up to three
similar lines from the current file. Agents can reread those lines and retry with
exact text. The bounded search can omit long or distant lines.

In code, call the same tool API (requires `everruns-integrations` feature
`filesystem`):

```rust
use everruns_integrations::filesystem::EditFileTool;
use everruns_contracts::runtime::{tool_context::ToolContext, tools::Tool};
use serde_json::json;

# async fn edit(context: &ToolContext, hash: &str) {
let result = EditFileTool.execute_with_context(json!({
    "path": "/workspace/settings.rs",
    "expected_hash": hash,
    "edits": [{
        "old_text": "let enabled=false;",
        "new_text": "let enabled = true;"
    }]
}), context).await;
# }
```

A substantial, unique whole-line replacement already in the file produces a
possible already-applied hint, not a success result. Every failed batch leaves the
file unchanged. Exact matching, read permissions and concurrent-write protection
remain in place. See [File System](/capabilities/file-system/).
