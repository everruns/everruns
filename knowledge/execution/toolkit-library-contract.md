---
type: Specification
title: "Toolkit Library Contract"
description: "Convention for external toolkit libraries."
tags:
  - everruns
  - execution
---
# Toolkit Library Contract

## Purpose

External `*kit` crates expose standalone tools that Everruns can integrate
without hardcoding their schemas, prompts, execution details, or errors.

This is a behavioral convention, not a shared Rust trait crate. Exact public
items and signatures belong to each toolkit's source and generated API docs:

- [bashkit on docs.rs](https://docs.rs/bashkit) and
  [`integrations/bashkit/src/lib.rs`](../../integrations/bashkit/src/lib.rs)
- [fetchkit on docs.rs](https://docs.rs/fetchkit) and
  [`integrations/web-fetch/src/lib.rs`](../../integrations/web-fetch/src/lib.rs)
- the pinned versions and enabled features in
  [`crates/core/Cargo.toml`](../../crates/core/Cargo.toml)

Do not copy a released toolkit's Rust definitions into this spec. Toolkit APIs
evolve independently, and the pinned dependency plus integration tests are the
truth for the version Everruns actually consumes.

## Why the convention exists

Without a common shape, every toolkit integration must duplicate metadata,
input schemas, prompts, execution adapters, and error classification. That
duplication drifts when the toolkit adds a parameter or changes behavior.

The integration should instead delegate toolkit-owned concerns to the toolkit
and own only Everruns-specific policy and adaptation.

## Responsibility model

Toolkits separate three responsibilities:

1. A builder captures stable and kit-specific configuration.
2. A tool exposes immutable metadata and creates executions.
3. An execution represents one stateful, single-use tool call.

The exact type names may differ when an established toolkit has a better native
API. What matters is that consumers can obtain metadata without executing,
create isolated executions, and inject host adapters where needed.

Builders should be reusable: producing a schema, definition, service, or tool
must not unexpectedly consume configuration that another artifact needs.
Toolkit-specific configuration remains fluent and discoverable.

## Metadata contract

A toolkit is the source of truth for:

- stable LLM-facing tool name;
- localized display name and description;
- semantic version;
- input and output schemas;
- semantic hints such as read-only, destructive, idempotent, open-world,
  secret-dependent, or long-running behavior;
- the minimal system-prompt contribution;
- comprehensive user-facing help.

Everruns wrappers must delegate those values and add only platform-specific
extensions. For example, the bash wrapper may add a session working-directory
parameter because that concept belongs to Everruns, but it must not copy the
rest of bashkit's schema.

Tool objects must be safe to share across concurrent runtime assembly. Mutable
per-call state belongs to an execution, not the metadata object.

### Token economy

Descriptions and prompt contributions are paid on every model call. Names and
JSON Schema keywords should carry the obvious meaning. Use enums, formats, and
defaults instead of repeating the same information in prose. Add a short
description only for a non-obvious constraint or behavior.

The system-prompt contribution contains only behavior the model cannot infer
from the schema. It has no decorative title and may be empty.

Comprehensive usage information belongs in toolkit help or product
documentation, not in the always-present prompt.

### Localization

Locale affects human-readable display names, descriptions, help, prompt text,
and safe user-facing errors. It does not change the stable tool name, schema
property names, or version.

Unsupported locales must degrade according to the toolkit's documented locale
policy; wrappers must not maintain a second translation table for toolkit-owned
text.

## Schema contract

Input schemas are valid JSON Schema objects suitable for LLM tool definitions.
They adapt to builder configuration: disabled functionality does not linger as
an apparently usable parameter.

Output schemas describe the model-visible result, not host-only diagnostics.
The toolkit's tests must verify that schemas and runtime parsing agree.
Everruns integration tests should verify that the delegated schema reaches the
runtime unchanged except for intentional platform extensions.

OpenAI-compatible tool definitions or `tower::Service` adapters are useful
interoperability surfaces when a toolkit provides them, but their exact Rust
signatures are toolkit-owned.

## Execution contract

An execution validates or parses one call's arguments and runs at most once.
The final output is authoritative.

Long-running toolkits may support:

- cancellation that is idempotent and safe to signal while execution is
  running;
- partial output on cancellation;
- an informational stream obtained before execution starts.

Dropping an unsupported cancellation future is an acceptable fallback. A
toolkit must not advertise cancellation or streaming if its implementation
cannot honor the lifecycle.

Incremental output is for live rendering. Consumers must not reconstruct a
different final result by concatenating chunks.

## Adapter boundary

A toolkit that needs a filesystem, file saver, HTTP transport, or another
host-owned facility defines the adapter trait in the toolkit. The host
implements that trait and injects it at the execution boundary.

Adapters are:

- named for the capability they provide rather than for Everruns;
- safe to share across asynchronous tasks;
- expressed in toolkit-owned input, output, and error types;
- re-exported with all supporting types needed by implementers.

The integration must preserve host policy. Session filesystem access stays
inside the session filesystem adapter, and sanctioned outbound traffic stays
inside the host egress boundary. See [`egress.md`](../operations/egress.md).

## Errors

Toolkit errors distinguish safe, actionable caller errors from internal
failures.

Safe errors are concise, localized when appropriate, and explain how the caller
can correct the request. They do not expose dependency names, stack traces,
internal paths, credentials, or debug dumps of prompt-sized values.

Internal failures retain diagnostic detail for operator logs but are mapped to
a generic model-visible failure by the consumer. The integration uses the
toolkit's classification rather than matching error strings.

Interpreter-backed toolkits own output hygiene at the source. Everruns may add
defense-in-depth truncation, but that does not justify verbose debug rendering
inside the toolkit.

## Output separation

Execution output has two audiences:

- model-visible result content and supported native media;
- host-only operational metadata such as duration, status, bytes transferred,
  redirects, command counts, or filesystem activity.

Host-only metadata may feed traces, events, billing, or UI diagnostics. It must
not be serialized into the model's tool result unless the toolkit explicitly
defines it as part of the result contract.

Everruns maps native images into model content blocks and maps ordinary results
into the runtime's success type. The exact output structs are defined by the
pinned toolkit versions and should not be repeated here.

## HTTP request signing

HTTP-capable toolkits support optional RFC 9421 message signatures using the
Web Bot Authentication profile. Non-HTTP toolkits are exempt.

The durable requirements are:

- signing is feature-gated so unused cryptography is not compiled;
- the host can provide an Ed25519 seed, optional agent identity, and validity
  window;
- signed requests cover the authority and carry the profile's algorithm,
  key identity, tag, nonce, and timestamps;
- public-key/JWK derivation is available to the host for discovery endpoints;
- signing is performed inside the toolkit before the request reaches an
  injected transport;
- invalid signing configuration disables signing with an operator-visible
  warning rather than making the underlying tool unavailable;
- secrets and private key material never enter tool output, events, or logs.

The exact config and key types belong to the toolkit. Fetchkit's current
implementation and [`fetchkit.md`](fetchkit.md) are the reference, while
[`docs/advanced/request-signing.md`](../../docs/advanced/request-signing.md)
documents operator-facing setup.

## Everruns integration rules

An Everruns capability wrapper should:

1. build the toolkit with capability configuration and locale;
2. delegate name, display metadata, schema, hints, and prompt contribution;
3. add only Everruns-owned schema or prompt behavior;
4. inject session filesystem or egress adapters at execution time;
5. map safe and internal errors through the toolkit's classification;
6. keep operational metadata out of the model-visible result;
7. test delegated schema and critical policy wiring against the pinned toolkit.

Current integration patterns live in the bashkit and fetchkit capability
modules linked above. Those modules, their tests, upstream toolkit docs, and the
lockfile are authoritative when this convention and a released API appear to
disagree.

## Independence

Toolkit crates do not depend on Everruns crates. Each kit releases
independently and may serve other runtimes. Everruns owns a thin adapter, not a
fork of the toolkit's schema or runtime.

A shared `toolkit-common` crate is intentionally not required. Consistency is a
design convention; forcing all kits onto one release cycle would defeat their
standalone role.
