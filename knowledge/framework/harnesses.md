---
type: Proposal
title: "Framework Harnesses"
description: "Promoting the harness to an application-facing value so an agent's world is separable from its behavior and from the machine it runs on."
tags:
  - everruns
  - framework
  - harnesses
  - rust
---

# Framework Harnesses

Status: proposed. Nothing below is implemented in `everruns`. It extends
[Execution environments](../harnesses/execution-environments.md) and answers the
half that concept left open, and it depends on work that concept also lists as
unbuilt.

## Problem

The hosted platform has a harness: a stored, org-scoped record that defines what
a session runs on, described in
[Harness Types](../harnesses/harness-types.md). The Framework has no equivalent.
`everruns` collapses harness, agent, and session into one `AgentBuilder`, and
synthesizes an anonymous `HarnessDefinition` per session when it assembles the
runtime (`crates/everruns/src/agent.rs`).

Three costs follow.

**Built-in worlds are written twice.** `generic` exists once in
`crates/server/src/harnesses/` for the platform and again as an ad-hoc builder
chain in every embedding application. The two drift, and nothing detects it.

**The world is not separable from the behavior.** A Framework agent attaches its
own shell and filesystem capabilities, so moving an agent from an in-process VFS
to a container means editing the agent. This is the coupling
[Execution environments](../harnesses/execution-environments.md) names as a
problem on the hosted side, where selecting an environment means selecting a
harness; the Framework reproduces it with no stored records to blame.

**Environment mismatch is discovered by the model.** An agent that needs native
processes, attached to a target that cannot run them, fails at the first tool
call rather than at configuration time. `ComputeCapabilities`
(`crates/host/src/compute.rs`) already records what a target can do and is
explicit that an unsupported operation must be absent rather than emulated, but
nothing compares it against what an agent requires.

## Model

Three layers, each answering one question.

| Layer | Question | Owns |
|---|---|---|
| **Environment** | Where do commands run, and what may they touch? | Workspace head, compute target, containment, durability |
| **Harness** | What am I running in, and what is available to me? | Required environment, capability set, model defaults |
| **Agent** | What role am I playing? | Instructions, function tools, hooks, model choice |

A Session binds one of each. This mirrors the hosted precedence rather than
inventing a second one: an Agent carries a harness reference
(`crates/platform/src/agent.rs`), and a session may override it.

Environment is not a new name. It is the resource name already decided for both
surfaces by [Execution environments](../harnesses/execution-environments.md),
and it already exists as a value with `compute`, `containment`, `capabilities`,
and `durability` members (`crates/host/src/workspace.rs`).

### The harness is narrower than the hosted record

A Framework harness carries the required environment, the capability set, and
model defaults. It carries neither a base system prompt nor starter files.

**No base prompt.** A written world-description drifts from the world it
describes, which is the failure
[Execution environments](../harnesses/execution-environments.md) documents when
it observes that the coding harnesses repeat near-identical prompts with
provider tool names spelled into the text. The environment already knows what it
can do; the preamble should be derived from `ComputeCapabilities` so it cannot
disagree with the target. This is an amendment to the hosted harness too, where
`system_prompt` is already optional.

**No starter files.** Starter files are a virtual-filesystem concept. When a
session is pointed at a real directory the file is already on disk, and writing
it either clobbers the user's copy or silently no-ops. Project instructions
reach the agent through the `agent_instructions` capability reading the
workspace, which works for both a VFS and a real directory.

What remains is a defensible object: a named set of capabilities plus the
environment they require.

### Requirements are negotiated, not assumed

A harness declares what the environment must provide. Session creation checks
that declaration against the environment's `ComputeCapabilities` and containment
level and fails with a typed error naming the missing capability.

This is the load-bearing property. Without it, "this harness needs a real
shell" is a comment; with it, the mismatch surfaces at configuration time and
names the specific gap.

`Environment` already rejects a containment profile weaker or stronger than its
target enforces (`crates/host/src/workspace.rs`). Harness requirements extend
the same idea one layer up.

### Harness is optional

`engine.create(agent)` with no harness behaves exactly as today, running on an
empty harness. The single-agent application never encounters the concept. A
harness is an opt-in binding in the existing session-builder chain beside
`environment`, not a second constructor.

### What can be shared with the platform, and what cannot

The split falls out of the type system rather than being chosen.

| | Serializes | Because |
|---|---|---|
| Harness | Yes | Capability references plus JSON config, model ids, requirement declarations |
| Agent | No | Function handlers, lifecycle hooks, and code-defined capabilities close over process-local state |

So one `generic` definition can feed both org provisioning and an embedding
application, while agent behavior stays code. This is the same reason local
persistence never serializes an Agent and requires an application to rebuild it
before resume, recorded in [Application API Boundaries](application-api.md).

An application still composes code-defined capabilities and tools on top of a
data-defined harness; the harness is the shared floor, not a ceiling.

## Naming

The concept is `Harness` on both surfaces. It was re-examined against the
alternatives and kept.

The strongest objection is real: elsewhere in the industry "agent harness" names
the agent loop. Anthropic's own guidance to agent authors classifies options by
which supplies "the harness (the agent loop + context management)". In Everruns
that loop is the Engine, and Everruns also uses the industry sense in its own
tagline, "durable agentic harness engine". A reader can meet both senses within
two pages.

Prior art offers no better word, because no other product has this object:

| Product | Machine | Behavior | Middle layer |
|---|---|---|---|
| Anthropic Managed Agents | `Environment` (reusable container template) | `Agent` (model, system, tools) | none |
| Vercel AI SDK | `Sandbox` (isolated microVM) | `Agent` (tools on the agent) | none |
| Codex CLI | `sandbox_mode` | `AGENTS.md`, model config | `profile` (named, layered overlay) |
| Everruns | `Environment` | `Agent` | `Harness` |

Everruns is alone in treating capabilities as first-class composable modules,
and that is exactly why the middle layer exists: `compaction`, `memory`, and
`budgeting` are neither tools the agent wields nor properties of a container, so
they cannot be folded into either neighbor the way the two-layer products fold
tools into the agent.

Rejected, with reasons:

| Candidate | Rejected because |
|---|---|
| `Preset` | Flattens the substrate reading; a preset has no boundary to defend and could contain anything |
| `Loadout`, `Kit`, `Toolset` | Imply contents the agent wields; `compaction` and `budgeting` are modules the agent is subject to |
| `Profile` | The nearest neighbour's term, but weak and already taken by `LocalProfile` |
| `Rig` | Accurate, but a register mismatch for a control-plane surface |
| `Runtime`, `Platform`, `Environment` | Taken; [Purpose and Terminology](purpose-and-terminology.md) forbids `Runtime` as a product name |

The cost of keeping the name is a definition the docs must state rather than
assume. That definition is the deliverable, not a rename: a Harness is what an
agent runs on; the Engine is what runs it.

## Success bars

- One `generic` definition serves both org provisioning and an embedding
  application, with a test that fails when they diverge.
- A harness requiring native processes, bound to a VFS target, fails at session
  creation with an error naming the missing capability, never at the first tool
  call.
- Moving an agent between a VFS target and a container changes the environment
  only. The agent and the harness are untouched.
- `engine.create(agent)` with no harness behaves exactly as it does today.
- No harness contributes hand-written prose describing its environment.

## Open questions

- **Does the hosted harness keep `system_prompt`?** This concept argues the
  derived preamble should replace it on both surfaces. The hosted record also
  carries `starters`, `intro_markdown`, and `icon`, which are presentation and
  stay hosted regardless.
- **Two named reusable templates.** If Environment becomes a stored resource as
  [Execution environments](../harnesses/execution-environments.md) proposes,
  Environment and Harness are both named, reusable, org-scoped records. The
  split is justified here on content, capabilities are not container config,
  but it is one more entity than the two-layer products, and the reason needs
  to be stated wherever both appear.
- **Ordering.** Requirement negotiation depends on environment profiles and the
  provider ports, which
  [Execution environments](../harnesses/execution-environments.md) lists as
  unbuilt. A Framework-first spike has no stored records or deprecation window
  to carry.

## Adjacent

`WorkspaceProvider` collides with the LLM `Provider`: `AgentBuilder::provider`
and `AgentBuilder::workspace_provider` sit in the same builder and mean
unrelated things, and `Durability::ProviderSnapshot` overloads the word again.
Renaming the workspace one clears the ambiguity without touching this design.
Tracked separately.

## Source index

- Agent composition and the synthesized harness: `crates/everruns/src/agent.rs`
- Session and environment binding: `crates/everruns/src/session.rs`,
  `crates/everruns/src/engine.rs`
- Environment, containment validation: `crates/host/src/workspace.rs`
- Compute targets and capabilities: `crates/host/src/compute.rs`
- Portable harness definition: `crates/host/src/builders.rs`
- Hosted harness record and agent binding: `crates/platform/src/harness.rs`,
  `crates/platform/src/agent.rs`
- Built-in harness definitions: `crates/server/src/harnesses/`

## See also

- [Execution environments](../harnesses/execution-environments.md)
- [Harness Types](../harnesses/harness-types.md)
- [Application API Boundaries](application-api.md)
- [Purpose and Terminology](purpose-and-terminology.md)
