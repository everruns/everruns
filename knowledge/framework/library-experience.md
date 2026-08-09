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
- multi-turn state is isolated by session and inspectable without backend
  access;
- live observation and cancellation never require ownership of the host event
  bus or task runner;
- awaited lifecycle extensions are explicit and distinct from non-blocking
  observation events;
- credentials, tenant records, and host lifecycle entities do not leak through
  model or agent values;
- optional integrations remain feature-gated and do not enlarge the default
  offline build;
- advanced host composition stays possible without making it the newcomer path.

Every promoted concern needs a facade-only, network-free acceptance fixture.
The exact fixture and parity rules are owned by [Application API
Boundaries](application-api.md); this document owns the user-experience outcome
they protect.

## Compatibility posture

Improving the Framework must not break the supported 0.17.x runtime path or
silently fork turn/provider semantics. Compatibility duration, host-only
allowances, and removal exclusions are specified in [Application API
Boundaries](application-api.md).
