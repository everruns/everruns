---
title: Scalable engines
description: Move Framework sessions from one process to PostgreSQL-backed workers without serializing Rust code.
---

`everruns::Engine` has two deliberately different implementations:

| Engine | Definition boundary | Persistence and restart | Best fit |
| --- | --- | --- | --- |
| `InMemoryEngine` | Any `everruns::Agent`, including Rust closures and embedded drivers | Process-local | Libraries, CLIs, tests, and one-process applications |
| `everruns_scale::ScaleEngine` | `PortableAgent` containing stable registration paths only | PostgreSQL catalog, canonical events, Environment bindings, and durable run state | Multiple workers and process restart |

An arbitrary `Agent` is not serializable. It can close over application state,
hold a provider driver, install an event sink, or contain executable hook and
tool code. `ScaleEngine::try_create(agent)` therefore returns a typed
`EngineCreateError::PortableAgentRequired` before creating session or workflow
state. Use `ScaleEngine::submit` instead.

## Define references, register implementations

```rust
use everruns_scale::{PortableAgent, RegistrationPath};

let definition = PortableAgent::builder(
    "Answer concisely.",
    "gpt-5-mini",
    RegistrationPath::new("providers/openai/default")?,
    RegistrationPath::new("workspaces/project/v1")?,
)
.tool(RegistrationPath::new("tools/weather/v1")?)
.capability(RegistrationPath::new("capabilities/files/v1")?)
.hook(RegistrationPath::new("hooks/audit/v1")?)
.build()?;
# Ok::<(), everruns_scale::PortableAgentError>(())
```

At process bootstrap, create a `Registry` and register the implementation for
every referenced path. Workers must use the same path contract, although their
concrete implementations and credentials can come from deployment
configuration. Initialize `ScaleEngine::new(pool, registry)` and submit the
definition asynchronously.

Registration paths contain only ASCII letters, digits, `.`, `_`, `-`, and `/`.
They are bounded, versionable application identifiers—not URLs, secrets, or
Rust symbol names.

## Portability matrix

| Agent component | InMemoryEngine | PortableAgent / ScaleEngine |
| --- | --- | --- |
| Provider | Embedded `Provider` or bundled simulator | Required stable provider path; driver stays in `Registry` |
| Function tool | Async function or closure | Stable tool path; closure stays in `Registry` |
| Capability | Reference or code-defined implementation | Stable capability path; installer stays in `Registry` |
| Lifecycle hook | Async closure | Stable hook path; closure stays in `Registry` |
| Workspace / Environment | Memory, directory, or provider object | Required workspace path; registered implementation must reopen persisted heads |
| Event sink | Application object | Engine deployment configuration only; never in the definition |
| Runtime driver or arbitrary code | Allowed | Rejected; no portable-definition API exists |

Missing and duplicate registrations fail closed with `PortableAgentError`. The
error names both the component kind and the exact registration path. Definition
validation and registry resolution complete before Scale writes catalog or run
state.

## Restart and event authority

Scale owns versioned migrations that can initialize a blank PostgreSQL
database. The schema stores the portable catalog, complete canonical event
envelopes, exact session-to-Environment bindings, and the generic durable
workflow substrate. A restarted engine loads the same definition, resolves its
paths from the new process's registry, reopens the recorded WorkspaceHead, and
projects history from the canonical event log.

There is one conversation write authority: Scale's event log assigns event IDs
and per-session sequences transactionally. Worker scheduling records do not
create a second transcript. Steering joins the active durable run, cancellation
marks it terminal, and process restart resumes from persisted workflow events.

The registered workspace implementation must make the referenced head
available to every eligible worker—for example through a shared provider or a
shared filesystem selected by trusted deployment configuration. A worker that
cannot resolve a registration or reopen the exact binding fails the resume; it
does not substitute a default workspace.

## Migrating an embedded application

1. Inventory every provider, tool, capability, hook, and workspace used by the
   local Agent.
2. Give each implementation a stable, versioned registration path and register
   it during every worker's bootstrap.
3. Replace the raw `Agent` value with a `PortableAgent` containing those paths.
4. Initialize Scale against PostgreSQL, run its migrations, and submit new
   sessions through `ScaleEngine::submit`.
5. Keep existing local session IDs with their original `InMemoryEngine` or local
   persistence owner. There is no implicit conversion of captured Rust code or
   local workspace state into a portable session.

Portable definitions must not contain credentials. Resolve secrets when the
registered provider or tool implementation is constructed, protect the Scale
tables as application data, and scope each Engine instance to one application
trust domain.
