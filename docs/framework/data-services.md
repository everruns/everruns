---
title: Data services
description: Use persistent session SQL, named memory directories, and application-owned citable knowledge retrieval in Framework agents.
---

Data services attach to real Engine tool contexts. They require no Server database. Choose storage directories outside the agent workspace so filesystem tools cannot rewrite service state.

## Persistent SQL

Enable `sql-database` and add `SqlDatabase::open(directory)`:

```rust
use everruns::{Agent, Model, database::SqlDatabase};

let agent = Agent::builder()
    .instructions("Use SQL to retain structured notes.")
    .model(Model::new("gpt-6-luna", everruns::OpenAI::from_env()?))
    .sql_database(SqlDatabase::open(".private/session-databases")?)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

This enables `sql_execute`, `sql_query`, and `sql_schema`. Databases are isolated by Engine session identity; retaining a SQL directory alone does not retain Engine sessions. Reopen your Engine's durable hosting profile and reconstruct its agent registrations to resume the same sessions.

The local backend uses the Server's SQLite authorizer, statement execution, validation, row/byte limits and timeouts. `ATTACH`, extension loading, unsafe pragmas and virtual tables are denied. File locking coordinates quota checks across independent store instances/processes. Database names cannot select host paths. This is an agent-tool boundary; it does not isolate arbitrary host shell processes from storage directories.

## Named memory

```rust
use everruns::{Agent, Model, MemoryDirectory};

let agent = Agent::builder()
    .instructions("Read policy from /memory/policy; save notes in /memory/notes.")
    .model(Model::new("gpt-6-luna", everruns::OpenAI::from_env()?))
    .capability("session_file_system")
    .memory(MemoryDirectory::directory("policy", "./policy").read_only())
    .memory(MemoryDirectory::directory("notes", "./notes"))
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Names are distinct path components below `/memory/`. The application decides which sessions share a directory. Read-only mounts reject creation, editing, deletion and directory creation through every filesystem tool, while keeping host file permissions unchanged. Directory bindings are trusted host configuration. They provide no containment for tools that execute arbitrary native host processes.

## Knowledge bases and indexes

Enable `knowledge`. `Knowledge::documents` supplies a small deterministic keyword retriever; `Knowledge::directory` snapshots Markdown and text files with bounded enumeration and byte limits. Symlinks inside the snapshot are rejected.

```rust
use everruns::{Agent, Model, knowledge::{Knowledge, Document}};

let source = Knowledge::documents("support", vec![Document::new(
    "Refund policy", "Refund requests are accepted within 30 days.",
    "https://example.com/policy/refunds",
)]);
let agent = Agent::builder()
    .instructions("Search policy before answering and cite the source URI.")
    .model(Model::new("gpt-6-luna", everruns::OpenAI::from_env()?))
    .knowledge(source)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`search_knowledge` returns ranked passages with immutable file/version identities, source URIs and optional locations. `browse_knowledge` pages through bound originals and metadata; read and grep them under `/knowledge/<collection-id>/`. Only the application-bound source is accessible. For semantic/vector search, implement `Retriever::search` and return `Passage` values through `Knowledge::new`. Attach originals with `.catalog(documents)` when the provider should also support browsing and mounted reads. The retriever owns embeddings, refresh and provider billing. Result counts, snippets and citation metadata are bounded, and provider errors are sanitized. Enable `.automatic(true)` to retrieve source evidence before model execution; it is retained by input ID for replay in protected host service storage.

The [complete data example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/service_data.rs) runs a real agent that retrieves policy, persists a SQL decision and writes named memory:

```bash
cargo run -p everruns --features openai,sql-database,knowledge --example service_data
```
