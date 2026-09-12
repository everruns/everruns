---
type: Specification
title: "Command Tree Specification"
description: "The `everruns <noun> <verb>` command surface shared by the scripted MCP toolset, session shells, and Framework hosts."
tags:
  - everruns
  - execution
  - cli
---
# Command Tree Specification

## Abstract

Everruns exposes the same operations through several surfaces, and they had
drifted into two grammars: the domain-command catalog spelled them
`list_agents`, while the external CLI spelled them `everruns agents list` over a
separate hand-written command set. This concept defines the **command tree**:
one noun-verb grammar, declared once per command, rendered by every host.

The tree is a *spelling and a discovery surface*. It is not a second execution
path: a command's flat wire name remains its identity, and every invocation
still runs through `Command::run`, with its schema, policy check, and error
handling unchanged.

## Motivation

Three surfaces reach the same operations. Two of them, the `/mcp` endpoint and
the `platform` capability, already shared everything through
`platform_command_surface`. The external CLI did not, and covered roughly ten
hand-written noun groups against a catalog of hundreds of registered commands:
an external caller could not reach most of the product, while a caller inside a
session could.

A second, smaller motivation turned out to matter more. A flat namespace of
hundreds of commands cannot afford `--help`, because rendering every schema
exceeds the output limit; the catalog surfaces therefore forbade it and
required exact discovery instead. That constraint is a property of flatness,
not of help. A tree bounds every help response structurally.

## Required behavior

1. **Membership is opt-in.** A command joins the tree only by declaring
   `Command::cli()`, which defaults to `None`. Internal plumbing cannot become
   an agent-facing command by being written.
2. **The shape is declared, not inferred.** A route carries a `path` slice plus
   a `verb`, because flat names hide a hierarchy (`list_session_participants`
   is `sessions participants list`) and string surgery is wrong for exactly the
   irregular names that matter (`set_default_agent_version`,
   `diff_agent_versions`).
3. **The wire name stays the identity.** Flat spellings keep working as
   aliases, and dispatch, schema coercion, authorization, and error
   sanitization are the same code they were before the tree existed.
4. **Help is bounded by shape.** The root lists nouns, a node lists its
   children, a leaf renders its own flags and examples. No level needs a cap.
   Catalog discovery remains for search across the whole catalog.
5. **A host advertises only what it can serve.** A host that supplies no
   commands gets no `everruns` builtin and contributes no prompt text claiming
   one.
6. **Verbs follow the object, and never collapse two authorizations into one.**
   `delete` (archive) and `destroy` (permanent) stay separate verbs because
   they carry different policies; a flag that silently escalates authorization
   is the wrong affordance.

## Two adapters, one grammar

Hosts differ in what they can observe, not in what they mean:

| Host | Why | Adapter |
|---|---|---|
| Plain `bash` tool (sessions, Framework applications) | a builtin receives raw argv | walks the tree directly |
| `ScriptedTool` (`/mcp` endpoint, `platform` capability) | the host parses `--flag` pairs before a builtin runs and never surfaces bare words | rewrites the tree spelling into the flat name at statement boundaries, before the interpreter sees it |

The rewrite is conservative in the same way the positional rewriter is: it
fires only at statement-start positions, preserves quoted regions and escapes,
and stops consuming at the first flag-like token. Help wins over execution, so
`--help` anywhere in a command's flags prints help rather than reaching a
builtin that has no such flag.

## Where commands come from

`CliCommandSource` is the seam between the tree and the host's operations. The
server backs it with its registered domain-command catalog; a Framework
application backs it with whatever it owns, with no control plane, database, or
catalog involved. This is why the contract lives in the bashkit integration
rather than in the server: a tree that only a server could source would not be
one grammar across surfaces.

See [`integrations/bashkit/src/cli.rs`](../../integrations/bashkit/src/cli.rs)
for the contract, `crates/server/src/api/mcp_endpoint/cli_tree.rs` for the
inventory-backed source, and
[`examples/framework-cli-host`](../../examples/framework-cli-host) for a host
with no server behind it.

## Discoverability

A CLI, unlike a tool schema, does not advertise itself; nothing in a model's
context implies the tree exists. Three pointers state it once each: the
scripted tool's description names the shape and its help, catalog discovery
carries the spelling in a `cli` field, and rendered usage shows the tree
spelling so the line a caller copies is the one to type. The bash-tool adapter
contributes the interpreter's `llm_hint`.

This is the design's central bet, so it is measured rather than assumed. The
`cli-tree` cases in `evals/platform-capability` include deliberate
*discoverability signals*: because flat names still work, a model that ignores
the tree still answers correctly and fails those cases. A failure there means
the pointers weakened, not that the surface broke.

## Status

Implemented for the first tranche: `agents` (including `versions`), `sessions`,
`skills`, and `mcp-servers`. The remaining catalog categories opt in one at a
time. The external `everruns` binary still runs its own hand-written commands;
moving it onto this tree is the step that makes the grammar literally uniform
across all three surfaces.

## Related

- [Capabilities Specification](capabilities.md) — the `platform` capability and its catalog surface.
- [Bashkit Requirements](bashkit-requirements.md) — the shell the tree is served through.
- [Threat model](../security/threat-model.md) — `TM-BASH`, `TM-TOOL`.
