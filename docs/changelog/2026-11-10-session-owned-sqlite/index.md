---
title: "Local sessions use separate SQLite databases"
description: "Yolop and the Rust Framework isolate each local session, including children, so unrelated sessions do not contend on one SQLite writer."
date: 2026-11-10
kind: fix
significance: notable
products: [yolop, framework]
surfaces: [cli, code]
links:
  docs: [/framework/hosting/#sqlite-per-session]
---

Yolop now stores each local session in a separate SQLite execution database,
including child sessions, directly at `sessions/<session-id>/engine.sqlite`.
History search and resume read SQLite directly without maintaining a duplicate
`events.jsonl`. Background artifacts use the visible `background/` folder
inside the session. Coordinators retain child registrations while each
child owns its history, tasks and schedules. A busy session no longer takes
the SQLite writer lock for unrelated sessions.

Existing shared authorities and earlier per-session directory layouts transfer from canonical SQLite state, preserving
pending inputs, task messages, schedules and sealed secrets. Transfer acquires
exclusive ownership of the complete source family before the first snapshot,
so late child reports survive handoff, and refuses live execution leases. The original database stays available for backup.

With the Rust Framework's `local` feature:

```rust
use everruns::{SessionId, hosting::SqliteHosting};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _hosting = SqliteHosting::open_session("sessions", SessionId::new())?;
    Ok(())
}
```

See [SQLite per session](/framework/hosting/#sqlite-per-session) and
[Yolop session history](https://github.com/everruns/yolop#session-history).
Yolop carries the same upstream code until an aligned Framework release
includes it. Resume transferred sessions with the updated host.
