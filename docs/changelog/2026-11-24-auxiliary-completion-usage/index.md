---
title: Completion reviews now contribute to session usage
description: Tool-less session completions retain provider usage and cost without adding conversation messages, including streamed and unreadable review responses.
date: 2026-11-24
kind: fix
significance: minor
products: [framework, platform]
surfaces: [code, agent]
links:
  docs: [/framework/session-controls/]
---

Completion reviews and other tool-less session calls now contribute reported
usage and cost to the session's generation ledger. Previously, these calls
could incur provider charges absent from recorded totals.

```rust
// Cargo: everruns with default features.
# async fn review(session: &everruns::Session) -> Result<(), everruns::SessionControlError> {
let verdict = session
    .complete_out_of_band(Some("Answer yes or no."), "Was the request completed?")
    .await?;
# let _ = verdict;
# Ok(())
# }
```

[Session review calls](/framework/session-controls/)
retain accounting without conversation messages or private review content.
Usage survives session reopen and is retained even when a paid response is
unreadable. Streams record usage before their final chunk. Calls cancelled
before final provider metadata cannot recover unknown charges, and older
session totals are not backfilled.
