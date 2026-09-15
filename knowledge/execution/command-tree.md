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

## One command line, two surfaces

`everruns agents list --limit 10` means the same thing typed in a terminal and
typed in an agent's shell: the same flags, the same short options, the same
positionals, the same help. That only holds if there is one definition, so
there is one, in
[`everruns-cli-contract`](../../crates/cli-contract): the grammar as data, and
the single `clap::Command` builder both surfaces call.

The CLI's clap derive cannot be that definition. The control plane routes 50
commands and the CLI hand-writes a dozen of them, so deriving the rest would
mean hand-writing structs whose fields already exist as the parameter types
commands deserialize; and the agent-facing tree is assembled from commands
resolved at runtime, which a `&'static` derive tree cannot be. So a command
declares the part a schema cannot know — that `--harness` is worth a `-H`, that
`agents get` reads better with the id as a bare word, what a caller is trying to
do when they reach for it — in its `CliRoute`, everything else comes from the
parameter schema it already publishes, and both surfaces build from the result.

Where the two disagreed, the surface with users did not move: the shipped CLI's
spellings are pinned by a golden snapshot in `crates/cli/contract.golden`, and
the catalog's route is what changes.

The CLI mounts the contract commands it does not hand-write, dispatching them
through the `method` and `http_path` each one already declares, so it spells
every routed command without a hand-written implementation for each. A
hand-written command always wins where one exists: `everruns agents create`
reads a file, normalizes TOML the import endpoint cannot parse, and walks a
directory into `initial_files`, which no catalog command can do because the
catalog runs on the server and the files are here.

The contracts travel as a checked-in artifact, `crates/cli-contract/commands.json`,
because generation needs the command types in `everruns-server` and the CLI
cannot link them. A guard in the server asserts the artifact still matches
inventory. Fetching the catalog at runtime instead would make `everruns --help`
need a network round trip and a credential, which is the wrong trade for a CLI.

What this buys over hand-parsing `--flag value` pairs:

- **An unknown flag is an error.** The previous parsers kept `--limti 10` as a
  string property and passed it on, so a typo became a silently dropped
  argument. clap rejects it and names the flag that was meant.
- **A required field is enforced before dispatch**, with usage attached, rather
  than surfacing as a deserialization error from the far side.
- **A positional is declared, not faked.** `agents get agt_01h9` is an ordinary
  clap positional, so the statement-boundary pre-rewrite that inserts `--id`
  before a bare word has nothing to do on this path.

Two conventions are deliberate. A long flag is kebab-case and also answers to
the parameter's own snake_case name, because schemas are generated from Rust
structs while callers type kebab, and a script written against either should
keep working. And colour is pinned off: the workspace links clap with its
default features for the CLI, cargo unifies that across the build, so a command
that does not say `ColorChoice::Never` emits escape bytes into terminals and
tool results alike.

Short options are declared, never derived. Deriving from first letters collides,
and worse, shifts as fields are added, so a script written today would break
when an unrelated parameter appears beside it.

### Examples are part of the contract

Every routed command carries at least one worked example, and an example is an
intent and a command line, not a bare command line. `everruns sessions search
--query X` shows a caller syntax they could have guessed; "Search prior sessions
for an exact marker" tells them when to reach for it. An agent reading `--help`
is choosing between commands, not recalling one it already knows. The type asks
for both halves so the weaker form is not the easier one to write, and a guard
asserts every routed command has one.

A second guard parses every example against its own command. This is not
ceremony: when it was first switched on, twelve commands documented a bare-word
form no parser accepted, `agents analyze` and `agents preview` documented an
agent id when they take a draft configuration and no id at all, `agents import`
documented a `--definition` flag that does not exist, and `sessions stats` and
`sessions facets` documented a session argument they do not take. An example is
the line a caller copies, so it has to run.

### What each adapter gets

Raw argv is the whole difference. Where a host surfaces it, the tree resolves a
leaf and hands the rest of argv to that leaf's parser. Where it does not, the
`ScriptedTool` host's builtins are `ToolDef`s parsed by bashkit from the same
schema, and `ToolArgs` carries only the parsed parameters, so clap cannot do the
parsing there.

Help is different: rendering it needs no argv, so **both** adapters describe a
command in exactly the same words, from the same parser. Closing the remaining
parsing gap is an upstream change in
[`everruns/bashkit`](https://github.com/everruns/bashkit), because the
filesystem-less shell profile is `pub(crate)` and the path cannot be rebuilt
here on `Bash::builder()` without giving `execute` a filesystem it is
deliberately denied.

## Where commands come from

`CliCommandSource` is the seam between the tree and the host's operations. The
server backs it with its registered domain-command catalog; a Framework
application backs it with whatever it owns, with no control plane, database, or
catalog involved. This is why the contract lives in the bashkit integration
rather than in the server: a tree that only a server could source would not be
one grammar across surfaces.

The seam is deliberately clap-shaped rather than JSON-shaped: a source hands the
tree a `clap::Command` and gets back the `ArgMatches` clap produced, so the
bashkit integration never learns what an agent or an invoice is. A host writing
each command by hand passes a clap derive; the hosted product builds the same
values from its catalog.

See [`integrations/bashkit/src/cli/mod.rs`](../../integrations/bashkit/src/cli/mod.rs)
for the tree,
[`crates/cli-contract`](../../crates/cli-contract) for the grammar and its
schema-to-clap compilation, `crates/server/src/api/mcp_endpoint/cli_tree.rs` for the
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
