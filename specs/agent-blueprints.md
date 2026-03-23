# Agent Blueprints Specification


<!-- Design Decisions:
  - AgentBlueprint: code-defined agent template with private tools, baked-in prompt, fixed/default model
  - Contributed by capabilities via new `agent_blueprints()` trait method
  - Spawned via `spawn_subagent` tool (new `blueprint` + `config` params)
  - Child session stores `blueprint_id` — reason_activity and act_activity branch on it
  - Private tools: blueprint tools never leak to host agent's tool list
  - Fixed model: blueprint can hardcode model (e.g. Haiku for cheap scout work)
  - Config surface is narrow and typed: JSON Schema for allowed overrides
  - Runs on the same durable execution engine, same agentic loop, same event stream
  - First implementation: GitHubScout (read-only GitHub search, hardcoded to fast model)
-->

## Abstract

Agent Blueprints are pre-built agent definitions contributed by capabilities. Unlike subagents (which inherit the parent's harness + agent config), blueprints encapsulate a complete agent — prompt, private tools, model selection — behind a simple invocation interface. The host agent spawns a blueprint via `spawn_subagent`, but the blueprint controls its own internals.

Inspired by Claude Code's built-in subagents (Explore, Plan, Claude Code Guide) and AmpCode's Librarian pattern.

## Motivation

Current subagents inherit parent's `harness_id` + `agent_id`. In `reason_activity`, the child builds the same RuntimeAgent as the parent:

```
RuntimeAgentBuilder::new()
    .with_harness(parent_harness, ...)
    .with_agent(parent_agent, ...)
    .build()
```

This works for "do the same kind of work in parallel" (run tests, process files). It fails for specialist delegation where the child needs:
- **Different tools** — GitHub search tools the host doesn't have
- **Different model** — Haiku for cheap lookup work, not the host's Opus
- **Different prompt** — specialist instructions unrelated to the host's agent prompt
- **Tool privacy** — host shouldn't see or be tempted to call the specialist's internal tools

Blueprints solve this by providing an alternative RuntimeAgent assembly path.

## Design Principles

| Principle | Rationale |
|-----------|-----------|
| Blueprints are capabilities, not a new DB entity | Contributed via `Capability` trait. No new table. Session gains `blueprint_id` field to signal alternate assembly. |
| Same durable execution infrastructure | Same agentic loop (InputAtom → ReasonAtom → ActAtom). Same PostgreSQL workflows. Same event stream. Only RuntimeAgent assembly differs. |
| Private tools | Blueprint tools never appear in the host's tool list. They exist only in the child session's RuntimeAgent. |
| Fixed model is first-class | Cheap work should not burn expensive models. Blueprint author decides. |
| Narrow config surface | Host passes structured config validated against JSON Schema. Cannot override prompt or tools. |
| Discovery via system prompt | Available blueprints listed upfront so LLM delegates during reasoning, not via a discovery tool call. |

## Data Model

### AgentBlueprint

Returned by `Capability::agent_blueprints()`. Defined in code, not persisted.

See `crates/core/src/capabilities/mod.rs` for the trait definition.

| Field | Type | Description |
|-------|------|-------------|
| `id` | `&'static str` | Unique identifier (e.g. `"github_scout"`) |
| `name` | `&'static str` | Human-readable display name |
| `description` | `&'static str` | When to use this blueprint (LLM reads this for delegation decisions) |
| `model` | `BlueprintModel` | Model selection strategy |
| `system_prompt` | `&'static str` | Baked-in system prompt for the child agent |
| `tools` | `Vec<Box<dyn Tool>>` | Private tools — only available inside the blueprint's session |
| `max_turns` | `Option<usize>` | Iteration limit (default: 20) |
| `config_schema` | `Option<Value>` | JSON Schema for allowed host-provided config. `None` = no config accepted. |

### BlueprintModel

```rust
pub enum BlueprintModel {
    /// Always use this model. Host cannot override.
    Fixed(&'static str),
    /// Use this model unless host provides override via config.
    Default(&'static str),
    /// Use whatever model the host agent uses.
    Inherit,
}
```

`Fixed` — scout/lookup work uses cheap models regardless of host context. `Default` — power users can upgrade via config. `Inherit` — blueprints that need the host's reasoning level.

### Session Model Extension

New field on `Session`:

| Field | Type | Description |
|-------|------|-------------|
| `blueprint_id` | `Option<String>` | Blueprint ID. When set, `reason_activity` and `act_activity` build RuntimeAgent from the blueprint instead of from `harness_id`/`agent_id`. |
| `blueprint_config` | `Option<Value>` | Validated config passed by host at spawn time. |

This is the **key integration point** with the durable execution engine. The blueprint_id travels through the workflow — it's stored on the session row, not passed ephemerally.

## Capability Trait Extension

New method on the `Capability` trait with a default empty implementation:

```rust
/// Agent blueprints contributed by this capability.
/// Blueprints are pre-built agents with private tools and baked-in prompts.
fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
    vec![]
}
```

Additive — existing capabilities unaffected.

### Blueprint Registration

`CapabilityRegistry` collects blueprints from all registered capabilities:

```rust
impl CapabilityRegistry {
    /// All blueprints from all registered capabilities.
    pub fn blueprints(&self) -> Vec<&AgentBlueprint> { ... }

    /// Find a blueprint by ID across all capabilities.
    pub fn blueprint(&self, id: &str) -> Option<&AgentBlueprint> { ... }
}
```

Duplicate IDs across capabilities rejected at registration time.

## Execution: How It Actually Works

### Spawn (in `spawn_subagent` tool)

`SpawnSubagentTool::execute_with_context` gains a new branch:

```
if blueprint param present:
    1. Look up blueprint in CapabilityRegistry
    2. Validate config against blueprint.config_schema
    3. Create child session with:
       - parent_session_id = current session
       - harness_id = parent's harness_id (for infrastructure, not for RuntimeAgent)
       - agent_id = None (blueprint replaces the agent)
       - blueprint_id = blueprint.id
       - blueprint_config = validated config
       - subagent_name, subagent_task = as today
    4. PlatformStore.send_message(child_session_id, task)
    5. wait_for_idle(300s)
    6. Return result
```

Key difference: child session has `blueprint_id` set and `agent_id` cleared. This signals to the worker that RuntimeAgent assembly should use the blueprint path.

### RuntimeAgent Assembly (in `reason_activity`)

Currently `reason_activity` builds RuntimeAgent unconditionally from harness + agent. This changes:

```rust
// In reason_activity, after loading the session:
let runtime_agent = if let Some(ref blueprint_id) = session.blueprint_id {
    // Blueprint path: build from blueprint definition
    let blueprint = capability_registry.blueprint(blueprint_id)
        .ok_or_else(|| anyhow!("unknown blueprint: {}", blueprint_id))?;

    let model = match blueprint.model {
        BlueprintModel::Fixed(m) => m.to_string(),
        BlueprintModel::Default(m) => {
            // Check if config overrides model
            session.blueprint_config
                .as_ref()
                .and_then(|c| c.get("model"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| m.to_string())
        }
        BlueprintModel::Inherit => parent_model.clone(),
    };

    let mut prompt = blueprint.system_prompt.to_string();
    if let Some(ref config) = session.blueprint_config {
        // Inject config into prompt context
        prompt.push_str(&format!("\n<config>\n{}\n</config>", config));
    }

    RuntimeAgentBuilder::new()
        .system_prompt(&prompt)
        .tools(blueprint.tool_definitions())
        .model(&model)
        .max_iterations(blueprint.max_turns.unwrap_or(20))
        .build()
} else {
    // Standard path: build from harness + agent (unchanged)
    RuntimeAgentBuilder::new()
        .with_harness(harness, &registry, &ctx).await
        .with_agent(agent, &registry, &ctx).await
        .build()
};
```

### Tool Execution (in `act_activity`)

`act_activity` currently loads tools from harness + agent capabilities. For blueprint sessions, it loads tools from the blueprint instead:

```rust
// In act_activity, after loading the session:
let tools: Vec<Box<dyn Tool>> = if let Some(ref blueprint_id) = session.blueprint_id {
    let blueprint = capability_registry.blueprint(blueprint_id)?;
    blueprint.tools()  // Private tools only
} else {
    // Standard: load from harness + agent capabilities (unchanged)
    load_capability_tools(harness, agent, &registry)
};
```

This is where tool privacy is enforced. Blueprint tools are instantiated only inside the child's `act_activity`. The host's `act_activity` never sees them.

### Event Stream

Blueprint sessions emit the same events as any session:
- `subagent.spawned`, `subagent.completed`, `subagent.failed`, `subagent.cancelled` (lifecycle)
- `turn.started`, `turn.completed` (agentic loop)
- `output.message.*` (LLM output)
- `tool.started`, `tool.completed` (tool execution)

No new event types needed. The `subagent.spawned` event gains an optional `blueprint_id` field so the UI can display the blueprint name/icon.

### Statefulness and Follow-ups

The spawned session is **fully stateful** — real session with messages, events, durable workflow state. The host can:
- `message_subagent("Scout", "also check the auth middleware tests")` — sends a follow-up
- `get_subagents("Scout")` — checks status and reads messages

The `AgentBlueprint` definition is stateless (code template). The session instance is stateful.

## Invocation: spawn_subagent Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | Yes | Human-readable name (same as today) |
| `task` | string | Yes | Task description (same as today) |
| `blueprint` | string | No | Blueprint ID. When set, child uses blueprint's RuntimeAgent. |
| `config` | object | No | Blueprint-specific config. Validated against `config_schema`. |

## Discovery: System Prompt Contribution

The `SubagentCapability` system prompt contribution expands to include available blueprints:

```
<available-blueprints>
Specialized agents you can delegate to via spawn_subagent(blueprint: "<id>"):

- github_scout: Search GitHub repositories for code, issues, and discussions.
  Fast read-only agent. Use for codebase exploration, finding patterns,
  understanding unfamiliar repos. Config: { "repos": ["owner/repo"] }
</available-blueprints>
```

Generated dynamically from `CapabilityRegistry::blueprints()`. Each entry shows `id`, `description`, and a summary of `config_schema`.

## Security Considerations

| Concern | Mitigation |
|---------|------------|
| Capability escalation | Blueprint tools cannot exceed the contributing capability's permissions. Scoped by API keys/connections. |
| Prompt injection via config | Config validated against typed JSON Schema. No free-form prompt override. |
| Model cost | `Fixed` prevents host from running expensive models for cheap work. |
| Tool leakage | Blueprint tools only instantiated in child's `act_activity`. Host's `act_activity` never loads them. |
| Nesting | Same constraint as subagents — blueprint children cannot spawn subagents or blueprints. |
| Resource exhaustion | `max_turns` limit on blueprint. Same 300s timeout as subagents. |
| Blueprint impersonation | `blueprint_id` validated against registry at spawn time. Invalid IDs rejected. |

## Concrete Example: GitHubScout

First blueprint implementation. Integration crate: `integrations/github-scout/`.

### Capability

```rust
pub struct GitHubScoutCapability;

impl Capability for GitHubScoutCapability {
    fn id(&self) -> &str { "github_scout" }
    fn name(&self) -> &str { "GitHub Scout" }
    fn description(&self) -> &str {
        "Pre-built agent for searching GitHub repositories."
    }

    // No host tools — all tools are private to the blueprint
    fn tools(&self) -> Vec<Box<dyn Tool>> { vec![] }

    fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
        vec![AgentBlueprint {
            id: "github_scout",
            name: "GitHub Scout",
            description: "Search GitHub repositories for code, issues, and discussions. \
                          Fast read-only agent for codebase exploration and pattern discovery.",
            model: BlueprintModel::Fixed("claude-haiku-4-5-20251001"),
            system_prompt: GITHUB_SCOUT_PROMPT,
            tools: vec![
                Box::new(SearchGitHubCodeTool),
                Box::new(ReadGitHubFileTool),
                Box::new(SearchGitHubIssuesTool),
            ],
            max_turns: Some(15),
            config_schema: Some(json!({
                "type": "object",
                "properties": {
                    "repos": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Repository list to scope searches (owner/repo format)"
                    }
                }
            })),
        }]
    }
}
```

### Private Tools

| Tool | Description |
|------|-------------|
| `search_github_code` | Search code across repos using GitHub code search API |
| `read_github_file` | Read a specific file from a GitHub repo (by path + ref) |
| `search_github_issues` | Search issues and discussions with filters |

These use the GitHub token from User Connections. Never visible to the host agent.

### Host Usage

```json
{
  "name": "spawn_subagent",
  "arguments": {
    "name": "Scout",
    "task": "Find how authentication middleware is implemented in the fastify repo.",
    "blueprint": "github_scout",
    "config": { "repos": ["fastify/fastify"] }
  }
}
```

Child session runs with Haiku, 3 GitHub tools, scout prompt. Host gets back a summary.

## Relationship to Subagents

| Aspect | Subagent | Blueprint |
|--------|----------|-----------|
| RuntimeAgent source | Inherited from parent's `harness_id` + `agent_id` | Built from blueprint definition |
| Session fields | `agent_id` = parent's, `blueprint_id` = None | `agent_id` = None, `blueprint_id` = set |
| Prompt | Parent's prompt + task as user message | Blueprint's baked-in prompt + task as user message |
| Tools | Parent's tools (same `act_activity` path) | Blueprint's private tools (alternate `act_activity` path) |
| Model | Parent's model | Fixed/Default/Inherit per blueprint |
| Config | None (host controls via task text) | Narrow, typed, validated JSON Schema |
| Agentic loop | Same (InputAtom → ReasonAtom → ActAtom) | Same |
| Durable execution | Same (PostgreSQL workflow) | Same |
| Event stream | Same (SSE events on child session) | Same |
| Lifecycle | Same (spawn/get/message tools) | Same |
| Nesting prevention | Same (cannot spawn children) | Same |
| Use case | Parallel work with same capabilities | Specialist delegation with different capabilities |

## Changes Required

### Session Model (`crates/core/src/session.rs`)

Add two fields:

```rust
/// Blueprint ID. When set, reason_activity/act_activity use blueprint
/// for RuntimeAgent assembly instead of harness_id/agent_id.
pub blueprint_id: Option<String>,

/// Blueprint config passed at spawn time. Validated against blueprint's config_schema.
pub blueprint_config: Option<serde_json::Value>,
```

Migration: add nullable columns `blueprint_id TEXT` and `blueprint_config JSONB` to sessions table.

### Capability Trait (`crates/core/src/capabilities/mod.rs`)

Add `agent_blueprints()` default method returning `Vec<AgentBlueprint>`.

Add `AgentBlueprint` and `BlueprintModel` structs.

### CapabilityRegistry (`crates/core/src/capabilities/mod.rs`)

Add `blueprints()` and `blueprint(id)` methods that aggregate across all registered capabilities.

### SpawnSubagentTool (`crates/core/src/capabilities/subagents.rs`)

- Add `blueprint` and `config` params to schema
- New branch: when blueprint is set, validate config, create session with `blueprint_id`/`blueprint_config`/`agent_id=None`
- System prompt contribution includes `<available-blueprints>` section

### reason_activity (`crates/worker/src/activities.rs`)

Branch on `session.blueprint_id`:
- If set: build RuntimeAgent from blueprint (prompt, model, tool definitions)
- If not: existing path (harness + agent)

### act_activity (`crates/worker/src/activities.rs`)

Branch on `session.blueprint_id`:
- If set: load tools from blueprint
- If not: existing path (harness + agent capabilities)

### Subagent Events

`subagent.spawned` event gains optional `blueprint_id` field.

## Implementation Path

### Phase 1: Core Infrastructure

1. `AgentBlueprint`, `BlueprintModel` structs
2. `Capability::agent_blueprints()` trait method
3. `CapabilityRegistry::blueprints()` / `blueprint(id)`
4. Session model: `blueprint_id`, `blueprint_config` fields + migration
5. `reason_activity` blueprint branch
6. `act_activity` blueprint branch
7. `spawn_subagent` blueprint parameter handling
8. System prompt blueprint discovery

### Phase 2: GitHubScout

1. `integrations/github-scout/` crate with 3 private tools
2. Wire GitHub API via User Connections
3. Register via `inventory::submit!`
4. Add to Generic harness capabilities

### Phase 3: Maturation

- Blueprint-scoped MCP servers
- Filesystem access modes (none / read-only parent / isolated)
- More blueprints: CodeReviewer, TestRunner, DocWriter

## Open Questions

1. **Filesystem access** — GitHubScout doesn't need it. CodeReviewer would want read-only access to the parent's session filesystem. Add `filesystem_access: FilesystemAccess` field (None/ReadOnly/ReadWrite) to AgentBlueprint in Phase 3.

2. **`get_subagents` response** — include `blueprint_id` in the response so the host knows which specialist it's talking to.

3. **Can a capability contribute both host tools AND a blueprint?** Yes. A `github` capability could give the host `create_github_issue` directly AND contribute a GitHubScout blueprint for research. Separate namespaces.

4. **Blueprint model resolution for `Inherit`** — needs the parent session's model. `reason_activity` can load the parent session via `parent_session_id` to resolve this.
