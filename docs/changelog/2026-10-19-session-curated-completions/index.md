---
title: Send host-written review completions on Framework sessions
description: Session::complete sends exactly the system prompts and messages a host chooses, tool-less, through the session's current provider and model.
date: 2026-10-19
kind: feature
significance: minor
products: [framework]
surfaces: [code]
links:
  docs: [/framework/session-controls/]
---

A host that reviews a Framework session's work, such as checking whether a task
is complete, can now send its own completion request instead of one built from
the session's full conversation. `Session::complete` sends exactly the request's
system prompts and messages with no tools, through the provider and model the
session's next model call would use. It tags the call with a `purpose` and
leaves history untouched.

```rust
use everruns::{CompletionRequest, Session};

async fn review(session: &Session, evidence: String) -> Result<String, everruns::SessionControlError> {
    session
        .complete(
            CompletionRequest::new()
                .system_prompt("Decide whether the task is complete.")
                .user(evidence)
                .purpose("completion_review"),
        )
        .await
}
```

Reserved metadata keys and tool messages are rejected, and an unavailable
provider fails without fallback. See [Session Controls](/framework/session-controls/).
