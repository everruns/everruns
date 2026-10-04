---
type: Specification
title: "Framework Harnesses"
description: "Application-facing harness values share portable capability presets with the hosted platform."
tags:
  - everruns
  - framework
  - harnesses
  - rust
---
# Framework Harnesses

Status: implemented. A Harness is what an agent runs on; the Engine is what runs it.

## Boundaries

The Environment owns the workspace, compute target and containment. The Harness selects the reusable capability foundation, environment requirements and lowest-precedence model default. The Agent owns behavior: instructions, function tools, hooks and model choice. A Session binds these values.

A framework Harness carries no hosted presentation, base prompt or starter files. Agent instructions remain on the Agent. Project instructions are read through the capability from the selected workspace. The hosted record additionally carries labels, icons and stored inheritance; the runtime projection excludes persistence bookkeeping.

## Shared presets

Base, Conversation, Worker Base and Worker consume [one preset definition](../../crates/contracts/src/capability/presets.rs). Hosted records inherit the parent live; [framework constructors](../../crates/everruns/src/harness.rs) flatten the same parent chain and retain capability configurations. Tests compare the two effective surfaces and exercise serialized round trips.

[Harness Types](../harnesses/harness-types.md) owns the level choices, opt-in features, default and Generic deprecation contract. Do not approximate those presets with a parallel builder list in examples. The deprecated framework Generic constructor preserves its legacy surface; it is not an alias to Worker.

A preset names capabilities but does not activate optional compile-time integrations. Applications enable the integrations their host needs. Unregistered references follow the framework's existing inert-reference behavior. An application may add code-defined capabilities and tools on the Agent; the harness is a foundation, not a ceiling.

## Binding and requirements

A framework Harness is optional. Creating a session from an agent without binding a harness retains the existing empty foundation. Constructors do not change that default.

A harness may declare compute and containment requirements. Session startup negotiates them against the selected Environment and fails with a typed error naming the missing capability or insufficient containment. Changing the execution target changes the Environment rather than the agent's role or instructions.

## Serialization

Portable harness configuration is data and serializes. Agent tools and hooks can close over process-local state and remain application code. Deserializing a Harness validates its definition and creates a fresh runtime association identity; serialized values do not restore live ownership relationships. Applications rebuild agents before resuming local persistence.

## Why retain the name

The broader industry sometimes calls the whole loop a harness. Everruns uses Engine for the loop and Harness for the reusable capability foundation. Capabilities include context management and budgeting as well as tools, so Toolset is too narrow. Environment is already the compute/workspace boundary. Keeping Harness requires defining its boundary consistently on both surfaces.

## Sources

- [Application-facing Harness](../../crates/everruns/src/harness.rs)
- [Hosted Harness](../../crates/server/src/records/harness.rs)
- [Portable runtime projection](../../crates/core/src/harness_definition.rs)
- [Hosted definitions](../../crates/server/src/harnesses/mod.rs)
- [Session binding](../../crates/everruns/src/session.rs)
- [Environment](../../crates/host/src/workspace.rs)

## See also

- [Harness Types](../harnesses/harness-types.md)
- [Execution environments](../harnesses/execution-environments.md)
- [Application API Boundaries](application-api.md)
