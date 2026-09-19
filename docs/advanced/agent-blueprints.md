---
title: Author an agent blueprint
description: Contribute a code-defined specialist agent from a capability, with a typed configuration contract the spawn path enforces
sidebar:
  order: 35
---

An **agent blueprint** is a code-defined specialist agent: a baked-in prompt, a set
of private tools, a model-selection strategy, an iteration bound, and a narrow
configuration surface. A host agent delegates to it through the ordinary
[sub-agents](/capabilities/sub-agents/) tool without gaining access to its
internals.

Reach for a blueprint when work needs different tools, instructions, or model
economics than the parent agent — repository scouting, catalog benchmarking, any
job where the parent should get the answer without carrying the tools that produced
it. Blueprints are not persisted user-created agents; they ship with a capability
and are available wherever that capability is enabled.

## Contribute the blueprint

A capability contributes blueprints by implementing `agent_blueprints()`. The
returned `AgentBlueprint` carries everything the child runtime needs:

```rust
fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
    vec![AgentBlueprint {
        id: "repo_scout",
        name: "Repo Scout",
        description: "Search repositories for code, files, and issues. \
                      Read-only agent for codebase exploration.",
        model: BlueprintModel::Fixed("claude-haiku-4-5-20251001".to_string()),
        system_prompt: REPO_SCOUT_PROMPT,
        tools: vec![Box::new(SearchCodeTool), Box::new(ReadFileTool)],
        max_turns: Some(15),
        config_schema: Some(json_schema_for::<RepoScoutConfig>()),
    }]
}
```

The `description` is what a parent agent reads when deciding whether to delegate,
so write it as a routing decision: what the blueprint is for, and when to pick it.

`model` chooses one of three strategies. `Fixed` pins a model the host cannot
override, which suits specialist work with a known cost/quality target. `Default`
names a model the validated config may override. `Inherit` takes the parent's
model, for work that genuinely needs the parent's characteristics.

Tools listed here are **private**. They are instantiated only for the blueprint's
own child session and never appear in the host agent's tool list.

## Define configuration as a type

Derive the config schema from a Rust struct rather than writing JSON by hand. The
struct is the single source of truth: field set, bounds, defaults, and descriptions
all reach the spawning agent from one place.

```rust
use everruns_capability::json_schema_for;
use everruns_capability::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The same ceiling the search tools apply to their own arguments.
const MAX_REPOS: u32 = 50;

/// Configuration for the repository scout.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(crate = "everruns_capability::schemars")]
pub struct RepoScoutConfig {
    /// Maximum number of repositories to scan.
    #[schemars(range(min = 1, max = MAX_REPOS))]
    pub max_repos: u32,
}

impl Default for RepoScoutConfig {
    fn default() -> Self {
        Self { max_repos: 10 }
    }
}
```

Three habits keep the contract honest:

- **Express each bound once.** Write it as a constant shared by the schema
  attribute and the code that enforces it at runtime, so a schema edit cannot
  drift from the clamp it describes.
- **Write doc comments for the caller.** They become the schema descriptions the
  spawning agent reads. Implementation notes belong in ordinary `//` comments.
- **Close the object** with `deny_unknown_fields` unless the blueprint genuinely
  accepts open configuration.

Requiring configuration is a matter of leaving a field without a default: fields
serde cannot default become `required` in the derived schema, and the spawn path
rejects a call that omits them.

## What the spawn path enforces

Configuration is validated against the derived schema **before** a child session
exists, so a declared bound constrains the host rather than merely advising the
model. A spawn is rejected when:

- a value violates the schema — out of range, wrong type, missing a required
  property;
- the config carries a key the schema does not define (with
  `deny_unknown_fields`);
- the blueprint declares no schema at all but config was supplied.

The error returned to the calling agent names the violations and includes the
schema, so a model can usually correct its own call and retry.

Validation governs what a **host** may configure. It is not a substitute for a
tool checking its own arguments: keep the runtime clamps in the blueprint's tools,
so a misbehaving child agent stays inside the same envelope.

Configuration can only select behavior the blueprint intentionally exposes. It
cannot replace the system prompt, inject tools, bypass model policy, or expand
capability permissions.

## Delegation and lifetime

A spawned blueprint session is a real durable session. It takes part in the same
message, event, task, workspace, cancellation, and recovery infrastructure as any
other sub-agent; the difference is only how its runtime is assembled. The session
persists the blueprint identity and its validated config, so a worker can
reconstruct the same runtime after a retry or handoff.

The blueprint itself stays a stateless template. Follow-ups address the durable
session, not the blueprint.

## Worked examples

Two blueprints ship in-tree and are worth reading as references:

- [`integrations/github/src/lib.rs`](https://github.com/everruns/everruns/blob/main/integrations/github/src/lib.rs) —
  the minimal case: one config field, a pattern constant shared with the runtime
  check that enforces it.
- [`integrations/openrouter/src/model_scout.rs`](https://github.com/everruns/everruns/blob/main/integrations/openrouter/src/model_scout.rs) —
  the fuller case: numeric bounds tied to runtime constants, a nested config type,
  and a spend budget.

## Related

- [Sub-agents](/capabilities/sub-agents/) — the delegation tool blueprints are
  invoked through
- [GitHub Scout](/capabilities/github-scout/) — a shipped blueprint from the
  caller's side
