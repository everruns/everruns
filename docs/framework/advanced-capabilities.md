---
title: Configure and author capabilities
description: Use one open AgentBuilder capability entrypoint for typed built-ins, dynamic references, and code-defined packages.
sidebar:
  order: 1
---

Every agent capability enters through `AgentBuilder::capability`. The method
accepts the public, non-sealed `IntoCapability` contract, so Framework built-ins
and third-party packages compose without adding a method or enum variant to
`AgentBuilder`.

Cargo features determine which environment-backed implementations a Framework
binary contains. See [Capability integrations](/framework/capability-integrations/)
for filesystem, Bashkit, web-fetch, Lua, and MCP boundaries; this page covers
agent-level configuration and authoring after an implementation is available.

## Configure capabilities

Use typed values when the Framework exposes a stable configuration, a
`capability::Definition` for application code, and `CapabilityRef` when the ID
and JSON arrive dynamically:

```rust
use everruns::{
    Agent, CapabilityRef, CompactionConfig, Model, ToolSearch,
};
use serde_json::json;

let weather_definition = build_weather_capability();
let agent = Agent::builder()
    .instructions("Use configured capabilities when relevant.")
    .model(Model::simulated("done"))
    .capability(CompactionConfig::new().budget_percent(0.85))
    .capability(ToolSearch::automatic())
    .capability(weather_definition)
    .capability(
        CapabilityRef::new("vendor.custom")
            .config(json!({ "mode": "database-driven" })),
    )
    .build()?;
```

`ToolSearch::automatic` uses hosted deferred loading on supported models and
the existing provider-neutral client-side implementation everywhere else. Its
optional threshold and never-defer allowlist map to the built-in's real
configuration; there are no provider fields on the Framework value.

`CapabilityRef` is the explicit escape hatch for database, plugin, or catalog
configuration. Its ID stays open. An unknown ID is retained as a reference but
contributes nothing until the selected host or plugin provides that
implementation. This is not a function tool: ordinary functions remain on
`AgentBuilder::tool` and `#[everruns::tool]`.

JSON capability config is not a credential store. Framework debug output
redacts it, but a host may persist or inspect it; pass a provider-owned secret
handle rather than API keys or tokens.

Conversion is infallible. `AgentBuilder::build` validates ID syntax and the JSON
object boundary, runs known built-in and declarative/plugin validators, and
rejects duplicate IDs after built-in alias resolution. A code implementation
cannot shadow a built-in or be paired with a second reference of the same ID;
registrations never use last-write-wins behavior.

Third-party typed values implement `IntoCapability` using only `everruns`:

```rust
use everruns::{CapabilityRef, CapabilitySpec, IntoCapability};

struct VendorSearch {
    index: String,
}

impl IntoCapability for VendorSearch {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new("vendor.search")
            .config(serde_json::json!({ "index": self.index }))
            .into()
    }
}
```

No `everruns-core`, registry, store, or host dependency is needed.

## Choose the standard policy bundle

The Framework's default `builtins` feature links `everruns-builtins`, the
backend-neutral implementation bundle for compaction, tool search, budgeting,
loop/progress safeguards, prompt caching, tool-call repair, output handling,
and guardrails. Linking the package has no registration side effect: each host
constructs its registry explicitly, so a custom registry cannot be changed by
dependency order.

Applications that want only the open Framework contracts can disable default
features and add the integrations they need. The policy bundle owns no network
client, process runner, interpreter, database, or hosted service. Output
persistence and distillation declare `session_file_system` as a host-provided
dependency; enable them only in a composition that supplies that capability.

## Choose an authoring level

Use the smallest extension contract that fits the behavior you own.

| Contract | `#[everruns::tool]` | `everruns::capability` |
|---|---|---|
| Best for | One application function | A reusable capability package |
| Typed input and result | Yes | Yes |
| Generated input schema | Yes | Yes |
| Inspectable output schema | No | Yes |
| Multiple tools | Register functions separately | One stable capability id |
| Capability instructions and metadata | No | Yes |
| Session/workspace identity and locale | No | Curated `Context` accessors |
| Progress events | No | `Context::progress` |
| Child-work cancellation | Turn future only | `Context::cancellation` |
| Backend/store/tenancy access | No | No |

## Ordinary tools

Annotate a typed async function. Its doc comment becomes the description and
its arguments become JSON Schema.

```rust
/// Convert Celsius to Fahrenheit.
#[everruns::tool]
async fn fahrenheit(celsius: f64) -> f64 {
    celsius * 1.8 + 32.0
}

let agent = everruns::Agent::builder()
    .instructions("Use the conversion tool.")
    .model(everruns::Model::simulated("done"))
    .tool(fahrenheit())
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

Prefer this until you need a capability-level contract.

## Advanced capabilities

An advanced capability is an immutable `capability::Definition`. It owns a
stable id, catalog text, optional instructions and JSON metadata, and one or
more typed handlers. `AgentBuilder::capability` installs its implementation on
the private in-process runtime and activates that stable id once.

```rust
use everruns::{Agent, Model, capability};

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct LookupInput {
    id: String,
}

#[derive(capability::Serialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct Record {
    id: String,
    score: f64,
    labels: Vec<String>,
}

struct Lookup;

#[capability::async_trait]
impl capability::Handler for Lookup {
    type Input = LookupInput;
    type Output = Record;
    type Error = capability::Error;

    fn name(&self) -> &str { "lookup_record" }
    fn description(&self) -> &str { "Look up one record by exact id." }

    fn hints(&self) -> capability::Hints {
        capability::Hints::default()
            .readonly(true)
            .idempotent(true)
    }

    async fn execute(
        &self,
        input: Self::Input,
        context: capability::Context,
    ) -> Result<Self::Output, Self::Error> {
        context.progress("Looking up the record").await;
        if input.id != "rec_42" {
            return Err(capability::Error::user(
                "record_not_found",
                "No record has that id",
            ).details(capability::serde_json::json!({ "id": input.id })));
        }
        Ok(Record {
            id: input.id,
            score: 0.98,
            labels: vec!["verified".into()],
        })
    }
}

let records = capability::Definition::new(
    "records",
    "Records",
    "Application-owned record lookup.",
)
.instructions("Use exact record ids and do not infer missing records.")
.metadata(capability::serde_json::json!({ "owner": "risk" }))
.tool(Lookup);

let agent = Agent::builder()
    .instructions("Answer with verified record data.")
    .model(Model::simulated("done"))
    .capability(records)
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

Both input and output types must satisfy the compile-time protocol bounds. The
generated schemas are available through `Definition::tools()[..].spec()` for
tests, documentation, or a host catalog. Results are serialized directly to
JSON, so structs, arrays, numbers, booleans, and null do not pass through a
string conversion.

## Errors

Return `capability::Error::user(code, message)` for an expected domain failure.
Add bounded JSON details when they help the model recover. The code, message,
and details travel through the model-visible tool-error channel.

Return `capability::Error::internal(code, message)` for diagnostic details that
are unsafe to show to the model, such as network internals or implementation
bugs. The engine logs the diagnostic and gives the model a generic error;
internal details do not cross the model boundary. Never include credential
values or other secrets in any error because host logs may retain internal
diagnostics.

Custom application error enums can implement `Into<capability::Error>` and be
used as `Handler::Error`.

## Context, progress, and cancellation

`capability::Context` exposes only stable lifecycle data:

- opaque session and workspace ids;
- the resolved locale, when present;
- best-effort correlated `tool.progress` events;
- a call-scoped cancellation signal.

Observe progress through `Session::events()` and
`SessionEventKind::ToolProgress`. Everruns does not currently expose a custom
capability result-streaming protocol; return one typed result when execution
finishes.

Normal awaited work needs no cancellation branch. Cancelling a turn drops the
handler future. Clone `context.cancellation()` only into child tasks, processes,
or watchers that might otherwise survive after `execute` is dropped. The signal
fires on cancellation and on every other call completion path.

## Security boundary

The advanced SPI intentionally does not export provider credentials, stores,
tenant or organization objects, registries, payment authority, filesystem
backends, or other host services. Pass application-owned clients or state into
your handler struct when constructing the definition. A handler is trusted
application code and retains whatever process authority those values provide;
`Hints` describe behavior but do not enforce authorization, egress, or approval
policy. Apply authorization, egress policy, timeouts, and input bounds at those
application boundaries, and never place secrets in capability or tool metadata.

For a complete provider-backed program, run the
[`advanced_capability` example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/advanced_capability.rs).
