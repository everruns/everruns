---
type: Specification
title: "Runtime Compatibility and Deprecation Policy"
description: "Ownership, signaling, and removal policy for the everruns-runtime 0.17.x bridge."
tags:
  - everruns
  - framework
  - compatibility
  - deprecation
---

# Runtime Compatibility and Deprecation Policy

## Decision

`everruns-runtime` is a compatibility-only bridge for existing 0.17.x
applications. It remains published, usable, and source-compatible for the full
0.17.x line and is removed in 0.18. It is not yanked during the transition and
owns no execution implementation.

Ordinary applications target the `everruns` Framework. Advanced system
integrators target `everruns` plus `everruns-host` and focused MCP, provider,
platform, or integration crates. Success is the absence of a runtime dependency
or import, not one-crate purity for a complete host.

Canonical host execution, events, replay, backend composition, filesystem
implementations, and low-level builder types live in `everruns-host`.
`everruns-runtime` may contain only re-exports, aliases, and isolated source
compatibility adapters. The event log is authoritative; legacy mutable-history
compatibility never returns to execution ownership.

## Deprecation signaling

The transition is signaled through the package description, README, crate-level
rustdoc/docs.rs landing page, and the single public migration guide. Those
surfaces state the replacement imports and 0.18 removal explicitly.

Do not apply a blanket Rust `deprecated` attribute to the crate or common
compatibility symbols in 0.17.x. A real external consumer compiled with
`-D warnings` turns that signal into an error, which breaks the supported bridge
instead of guiding it. The downstream migration fixture therefore compiles the
ordinary legacy path under `-D warnings` and rejects a crate-level attribute.

Narrow warnings are acceptable only for isolated APIs that are already outside
maintained execution and have exact replacements. The retained legacy shims
satisfy that condition; ordinary runtime construction and re-export paths do
not emit warnings.

## Documentation ownership

This specification owns the compatibility and deprecation policy. The public
`/framework/runtime-compatibility/` page owns migration instructions and exact
old-to-new mappings. Other public docs lead with the Framework and link to that
page only when they need to discuss the legacy runtime.

Guards forbid new primary documentation from recommending a runtime install,
forbid maintained source imports and dependency edges outside the explicit
compatibility allowlist, and keep the runtime crate logic-free. The publish
graph keeps `everruns-host` before `everruns-runtime`, and `everruns-runtime`
before `everruns`, for the remaining 0.17.x releases.
