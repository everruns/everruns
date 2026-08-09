---
title: Author tools and advanced capabilities
description: Choose #[everruns::tool] for ordinary functions or everruns::capability for reusable typed capability packages.
sidebar:
  order: 1
---

Everruns has two public Framework extension levels. Use the smallest one that
fits the behavior you own.

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
more typed handlers. `AgentBuilder::advanced_capability` installs it on the
private in-process runtime and activates it for the agent.

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
    .advanced_capability(records)
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
