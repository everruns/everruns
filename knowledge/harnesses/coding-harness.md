---
type: Specification
title: "Coding Harness"
description: "Provider-neutral coding behavior whose execution target is selected by an Agent Sandbox Template binding."
tags:
  - everruns
  - harnesses
  - environments
---
# Coding Harness

`coding` is an adoptable, provider-neutral harness example. It inherits Worker Base
and adds coding behavior plus `github_scout`; it does not select a compute
provider.

The Agent owns a Sandbox policy with named template bindings. A Session selects one binding
and receives the same model-facing tools on every supported target:
`bash`, `read_file`, `write_file`, `edit_file`, `glob`, and `grep`. The resolved
specification is pinned to the Session's logical Sandbox.

- A `vfs` profile retains Worker Base's session filesystem and Bashkit shell.
- A `managed` Daytona profile replaces both with `session_sandbox`, so shell and
  file tools address one remote `/workspace` rather than two filesystems.
- The harness prompt names neither providers nor provider-specific tools. The
  Sandbox preamble is derived from the resolved target and containment.

The legacy `coding-daytona`, `coding-container`, and
`coding-session-sandbox` names remain reconciliation-only. Existing org rows
are preserved as editable custom harnesses, but they are no longer offered as
examples.

See `crates/server/src/harnesses/coding.rs` for the definition and
[Sandbox Templates and Execution Targets](sandbox-templates.md) for profile selection and
runtime semantics.
