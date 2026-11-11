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

Yolop stores each session, including children, at
`sessions/<session-id>/engine.sqlite`. History search and resume read SQLite
without a duplicate `events.jsonl`. Background artifacts use the visible
`background/` folder. Coordinators retain child registrations; each child owns
its history, tasks and schedules. A busy session no longer locks unrelated sessions.

Earlier shared authorities and session-directory layouts transfer canonical
SQLite state, preserving pending inputs, task messages, schedules and sealed
secrets. Exclusive ownership of the complete source family precedes snapshots,
so late child reports survive handoff. Live execution leases refuse transfer;
original databases remain available for backup.

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
