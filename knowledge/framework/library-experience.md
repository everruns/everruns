---
type: Specification
title: "Framework Library Experience"
description: "Ergonomic and behavioral success bars for the application-facing Framework."
tags:
  - everruns
  - framework
  - developer-experience
---

# Framework Library Experience

## Success bars

The Framework succeeds when an application author can move from dependency to
useful agent behavior through one recognizable public crate and value-first
configuration. The smallest path is deterministic, offline, credential-free,
and independent of the Platform.

Application-facing behavior should have these qualities:

- common concepts are named in application language rather than storage or
  worker language;
- configuration errors surface before execution when the application can fix
  them;
- the common live-provider path reads as separate provider configuration and a
  plain model id, without exposing execution-facing model specifications;
- multi-turn state is isolated by session and inspectable without backend
  access;
- sessions accept messages independently of response timing, automatically
  route concurrent input to the active or next turn, and let applications wait
  only when they choose;
- live observation and cancellation never require ownership of the host event
  bus or task runner;
- awaited lifecycle extensions are explicit and distinct from non-blocking
  observation events;
- credentials, tenant records, and host lifecycle entities do not leak through
  model identity or diagnostic values;
- optional integrations remain feature-gated and do not enlarge the default
  offline build;
- advanced host composition stays possible without making it the newcomer path.

Every promoted concern needs a facade-only, network-free acceptance fixture.
The exact fixture and parity rules are owned by [Application API
Boundaries](application-api.md); this document owns the user-experience outcome
they protect.

## Compatibility posture

The Framework follows the semantic version of the `everruns` package rather
than a workspace-wide product version. Within a compatible release line,
improving the Framework must preserve its public application contract and must
never fork turn or provider semantics from the shared execution path. Removed
pre-0.18 runtime entrypoints are not a parallel compatibility target.

Application versus host ownership is specified in [Application API
Boundaries](application-api.md).
