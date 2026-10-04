---
type: Spec
title: Portable agent packages
description: One authored agent contract shared by Platform transports and Framework hosts.
tags: [agents, portability, framework]
---

# Portable agent packages

A package transfers authored behavior and assets, including complete skill trees.
The shared [codec](../../crates/core/src/agent_package/mod.rs) owns parsing,
normalization, bounded materialization and semantic diffs. UI, REST, CLI,
Platform Chat, MCP and Framework hosts reuse it; transports do not maintain
independent manifest formats. Existing simple Markdown remains an input form.
The codec is a module in `everruns-core`, following the existing
[crate layout](../project/crate-layout.md), rather than another published package.
`agent-package` enables in-memory formats, ZIP and virtual folder handling;
`agent-package-fs` separately enables native disk reads and writes. Core defaults
stay empty. The Framework includes disk loading by default and exposes the same
feature for applications that disable its defaults.
Public authoring instructions and field reference live in the
[product guide](../../docs/how-to/define-agents-as-files.md).

## Portability boundary

Exports use addressable names and provider-visible model names. Runtime resource
IDs, tenant identity, connection grants and executable host closures remain
host-owned. Imports resolve dependencies under the destination's existing
permission, feature and capability policies before mutation. Missing bindings
fail explicitly. A complete import replaces authored configuration; channel
intent is upserted, with new channels disabled by default and opt-in draft enablement.
Live changes use the same publication permission preflight as channel updates.
Existing activation and credential
bindings remain destination-owned, and omitted channel bindings survive.

Disk loaders read declared roots and conventional companion assets. ZIP is the
folder transport. Server commands read workspace paths through session file
commands, never through the server host filesystem. Collection rejects traversal
and symlinks and excludes hidden credentials. File bodies in diffs are represented
by digests; MCP credential requirements stay unresolved until explicitly bound.

## Execution and success bar

The [Framework adapter](../../crates/everruns/src/package.rs) and
[serve adapter](../../crates/serve/src/agent.rs) reuse the existing runtime.
Assets are frozen when loaded. A serve build digest includes package contents.
Channels describe ingress intent; hosts bind transports independently.

Round trips preserve instructions, execution settings, binary initial files and
all skill assets across text, folder and ZIP representations. Acceptance includes
executing imported definitions, reading bundled assets through runtime tools,
serving a real session, and comparing against the result after import. See the
[codec tests](../../crates/core/tests/agent_package.rs),
[Framework tests](../../crates/everruns/tests/facade/package.rs),
[Platform tests](../../crates/server/tests/domain/agent_packages_test.rs), and
[serve tests](../../crates/serve/src/wire_tests.rs).
