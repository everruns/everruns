---
title: Framework session data and sandbox tools
description: Add persistent session SQL, named read-only memory, citable application retrieval and E2B sandbox lifecycle tools to Framework agents.
date: 2026-11-15
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/data-services/, /framework/native-services/]
---

![Everruns Framework with E2B](./integration@2x.png)

Framework agents can use persistent session SQL, named memory directories and application-owned knowledge retrieval. SQL reuses the Server authorizer and execution limits; retrieval returns bounded passages with citation provenance.

With the `sql-database` Cargo feature:

```rust
use everruns::database::SqlDatabase;

let databases = SqlDatabase::open("private-session-databases")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

[Data services](/framework/data-services/) shows the agent bindings and storage isolation. The optional E2B binding adds create, execute, files, pause, resume and delete operations with application credentials; [native services](/framework/native-services/) includes the runnable sandbox example and cleanup responsibilities.
