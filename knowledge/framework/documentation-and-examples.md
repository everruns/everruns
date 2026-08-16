---
type: Specification
title: "Framework Documentation and Examples Contract"
description: "Ownership and success criteria for Framework public docs, crate docs, and runnable examples."
tags:
  - everruns
  - framework
  - documentation
---

# Framework Documentation and Examples Contract

## Public learning path

The public `docs/framework/` section is the canonical usage guide. It begins
offline, teaches `everruns` as the primary crate, covers the coherent
application lifecycle, and marks `everruns-host` as advanced host material.
The `everruns` README and rustdoc are concise entrances
to that same path rather than independent architecture narratives.

The public architecture guide names the concrete `everruns::Engine` as the
application session owner and separately explains the lower-level
`everruns-engine::Execution` host contract. It shows both immediate and durable
execution converging on one turn kernel and distinguishes volatile, local
crash-durable, and distributed Platform persistence.

Exact API shapes belong in source/rustdoc. Public guides link there instead of
freezing exhaustive fields or variants in durable knowledge.

## Runnable examples

Framework examples are maintained next to the `everruns` crate and import only
the public facade. Their local catalog owns exact filenames, commands, features,
and credentials. Public docs link to that inventory and explain why a reader
would choose an example.

At least one useful path must run without a network or provider credential.
Example inventory and compilation are CI-protected so navigation cannot point
at planned or unmerged programs. Host examples may remain beside
`everruns-host`, but must be labeled as low-level rather than as the normal
application entrypoint.

## Documentation integrity

Published crates carry a newcomer-readable README and compiled crate-level
rustdoc that lead users toward the Framework when appropriate. Repository
guards validate required structure and local ownership of public documentation
links without asserting brittle prose.
