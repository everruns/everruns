---
type: Specification
title: "Framework Purpose and Terminology"
description: "Audience, public identity, crate relationships, and the Framework/Runtime/SDKs/Platform boundary."
tags:
  - everruns
  - framework
  - terminology
---

# Framework Purpose and Terminology

## Purpose and audience

The **Everruns Framework** lets Rust application authors describe and run
agents without first becoming execution-host implementers. It is for libraries,
CLIs, desktop applications, services, and tests that want agent behavior as an
application concern: instructions, models, providers, tools, files, sessions,
observation, and controlled extension.

The primary public crate is `everruns`. A normal application begins there and
should not need to construct stored domain records, backend registries, worker
phases, or a control plane to run an agent.

## Canonical terms

- **Framework** means the application-facing `everruns` crate and its public
  library experience.
- **Runtime** means low-level host execution. `everruns-host` is the crate that
  implements it, not a synonym for Framework.
- **SDKs** are remote clients for a running Everruns server. They do not embed
  Framework execution in the client process.
- **Platform** is the control plane, server, workers, UI, durable storage, and
  deployment topology used to operate agents as a service.

Public material uses these names exactly. “Rust Framework” and “Runtime” are
not alternative product names for the Framework.

## Crate relationships

`everruns` composes the application contract over focused implementation
crates. Core owns shared agent and provider-facing values; provider owns the
lean model-driver abstraction; engine owns the abstract execution contract,
state machine, atoms, and deterministic turn planning; host owns the immediate
in-process driver, shared effect application, backend composition, event persistence,
and low-level in-process execution; local supplies optional local host state;
macros implements the tool attribute re-exported by `everruns`; platform owns
backend control-plane entities.

Those implementation relationships do not make the focused crates alternative
application entrypoints. Advanced hosts may depend on the focused crates they
need, while normal applications remain on `everruns`. The durable
promote-versus-host-only classification is owned by
[Application API Boundaries](application-api.md).
