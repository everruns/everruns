---
title: Runtime Compatibility
description: Migrate application code toward the everruns Framework while preserving everruns-runtime 0.17.x compatibility.
---

`everruns-runtime` remains published, supported, and usable throughout 0.17.x.
It serves two narrower purposes:

1. source compatibility for existing in-process applications; and
2. low-level execution-host composition while focused host ownership evolves.

It is not the recommended starting point for a new application. Normal library
users should depend on `everruns` and use the Framework.

## Application migration map

| Existing runtime concern | Framework path |
| --- | --- |
| Build a harness/agent/session graph for one application | `Agent::builder()` and `Agent::session()` |
| Construct provider-specific model records | credential-free `ModelSpec` plus an attached `Provider` or provider convenience |
| Register an application function tool | `#[everruns::tool]` or `FunctionTool` |
| Seed application files | `AgentBuilder::file` and `readonly_file` |
| Use one real workspace | `AgentBuilder::workspace` or feature-gated `LocalConfig` |
| Configure scoped MCP or a plugin | `McpServer` and `AgentBuilder::plugin` |
| Inspect next-turn context | `Session::inspect` |
| Observe and cancel a live turn | `Session::events` and `Session::run_with` |

Move application concerns first. Keep direct runtime imports only where code
really owns host backend traits, stored entities, platform definitions, phase
adapters, or orchestration lifecycle.

## Compatibility rules

- Do not remove or compiler-deprecate `everruns-runtime` as part of an application migration.
- Do not create a second provider-resolution or turn-execution path.
- Preserve existing 0.17.x runtime examples and fixtures as compatibility coverage.
- Treat writable JSONL message storage as compatibility machinery, not the persistence design for new Framework applications.
- Do not assume or document a post-0.18 cutover.

Low-level users can continue with the
[`everruns-runtime` crate documentation](https://docs.rs/everruns-runtime).
New application code should return to the [Everruns Framework](/framework/).
