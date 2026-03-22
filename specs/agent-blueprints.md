# Agent Blueprints Specification

<!-- Design Decisions:
  - AgentBlueprint: code-defined agent template with private tools, baked-in prompt, fixed/default model
  - Contributed by capabilities via new `agent_blueprints()` trait method
  - Invoked through existing spawn_subagent tool (new `blueprint` parameter)
  - Private tools: blueprint tools never leak to host agent's tool list
  - Fixed model: blueprint can hardcode model (e.g. Haiku 4.6 for cheap scout work)
  - Config surface is narrow and typed: blueprint declares JSON Schema for allowed overrides
  - Same invocation contract as subagents (spawn/get/message), different runtime assembly
  - First implementation: GitHubScout (read-only GitHub search, hardcoded to fast model)
-->

## Abstract

Agent Blueprints are pre-built agent definitions contributed by capabilities. Unlike subagents (which inherit the parent's agent config and run a parent-controlled prompt), blueprints encapsulate a complete agent — prompt, private tools, model selection — behind a simple invocation interface. The host agent spawns a blueprint the same way it spawns a subagent, but the blueprint controls its own internals.

Inspired by Claude Code's built-in subagents (Explore, Plan, Claude Code Guide) and AmpCode's Librarian pattern.

## Design Principles

| Principle | Rationale |
|-----------|-----------|
| Blueprints are capabilities, not a new primitive | Reuses existing Capability trait, RuntimeAgent builder, and subagent infrastructure. No new entity type in DB. |
| Private tools | Blueprint tools never appear in the host agent's tool list. They exist only inside the spawned child session. |
| Fixed model is first-class | Cheap/fast work (search, lookup) should not burn host's expensive model. Blueprint author decides. |
| Narrow config surface | Host can pass structured config (e.g. repos to search). Cannot override prompt or tools. Blueprint declares allowed config via JSON Schema. |
| Same invocation contract | `spawn_subagent(blueprint: "github_scout", task: "...")` — host doesn't need new tools to use blueprints. |
| Discovery via system prompt | Available blueprints listed in system prompt so LLM can decide when to use them during reasoning, not during tool execution. |

## Data Model

### AgentBlueprint

Returned by `Capability::agent_blueprints()`. Defined in code, not persisted.

| Field | Type | Description |
|-------|------|-------------|
| `id` | `&'static str` | Unique identifier (e.g. `"github_scout"`) |
| `name` | `&'static str` | Human-readable display name (e.g. `"GitHub Scout"`) |
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

**Rationale:** `Fixed` exists because scout/lookup work should use cheap models regardless of host context. A GitHub search agent doesn't need Opus. `Default` allows power users to upgrade when needed. `Inherit` preserves subagent behavior for blueprints that need the same reasoning capability as the host.

## Capability Trait Extension

New method on the `Capability` trait with a default empty implementation:

```rust
/// Agent blueprints contributed by this capability.
/// Blueprints are pre-built agents with private tools and baked-in prompts.
/// They are spawnable via `spawn_subagent(blueprint: "<id>")`.
fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
    vec![]
}
```

This is additive — existing capabilities are unaffected. Only capabilities that want to contribute blueprints implement it.

### Blueprint Registration

The `CapabilityRegistry` collects blueprints from all registered capabilities:

```rust
impl CapabilityRegistry {
    /// All blueprints from all registered capabilities.
    pub fn blueprints(&self) -> Vec<&AgentBlueprint> { ... }

    /// Find a blueprint by ID across all capabilities.
    pub fn blueprint(&self, id: &str) -> Option<&AgentBlueprint> { ... }
}
```

Blueprints are keyed by `id`. Duplicate IDs across capabilities are rejected at registration time (panic in debug, last-wins in release).

## Invocation: spawn_subagent Extension

The existing `spawn_subagent` tool gains one optional parameter:

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `name` | string | Yes | Human-readable name (same as today) |
| `task` | string | Yes | Task description (same as today) |
| `blueprint` | string | No | Blueprint ID. When set, child uses the blueprint's RuntimeAgent instead of inheriting parent's. |
| `config` | object | No | Blueprint-specific config. Validated against `config_schema`. Ignored when `blueprint` is absent. |

### Behavior When `blueprint` Is Set

1. Look up `blueprint` ID in `CapabilityRegistry::blueprint()`
2. Validate `config` against `blueprint.config_schema` (if schema exists). Reject invalid config.
3. Create child session with `parent_session_id` set (same as today)
4. Build a **new RuntimeAgent** from the blueprint:
   - System prompt: `blueprint.system_prompt` (NOT the parent's prompt)
   - Tools: `blueprint.tools` (NOT the parent's tools) — these are private
   - Model: resolved from `BlueprintModel` (Fixed/Default/Inherit)
   - Max iterations: `blueprint.max_turns` or default
5. Inject `config` values into system prompt or tool context as appropriate
6. Send `task` as first user message
7. Block on `wait_for_idle` (foreground mode, same as today)
8. Return result (same as today)

### Behavior When `blueprint` Is Absent

Identical to current behavior — child inherits parent's harness/agent config.

## Discovery: System Prompt Contribution

When blueprints are registered, the `SubagentCapability` (or a new aggregator) contributes to the system prompt:

```
<available-blueprints>
Specialized agents you can delegate to via spawn_subagent(blueprint: "<id>"):

- github_scout: Search GitHub repositories for code, issues, and discussions.
  Fast read-only agent. Use for codebase exploration, finding patterns,
  understanding unfamiliar repos. Config: { "repos": ["owner/repo"] }
</available-blueprints>
```

This is generated dynamically from `CapabilityRegistry::blueprints()`. Each entry shows `id`, `description`, and a summary of `config_schema` (if any).

**Rationale:** The LLM needs the menu upfront during reasoning. A `list_blueprints` tool would require a tool call before the LLM can decide — wasteful for a small, static list.

## Tool Encapsulation

Blueprint tools are **private** to the spawned session:

- They do NOT appear in `RuntimeAgent.tools` of the host agent
- They do NOT appear in the host's system prompt
- They are instantiated only when the blueprint is spawned
- They are destroyed when the child session completes

This means a capability can contribute both:
1. Tools for the host agent (via `tools()`) — e.g. `spawn_subagent`
2. Tools for a blueprint agent (via `agent_blueprints()` → `AgentBlueprint.tools`) — e.g. `search_github_code`

These are separate namespaces. A tool name can exist in both without conflict.

## Security Considerations

| Concern | Mitigation |
|---------|------------|
| Capability escalation | Blueprint tools cannot exceed the capability's own permissions. A blueprint contributed by `github_scout` capability can only access what that capability's API keys allow. |
| Prompt injection via config | Config is validated against typed JSON Schema. No free-form prompt override. |
| Model cost | `Fixed` model prevents host from accidentally running expensive models for cheap work. `Default` model documents the intended cost profile. |
| Tool leakage | Blueprint tools never enter host's tool list. Separate RuntimeAgent assembly path. |
| Nesting | Same constraint as subagents — blueprints cannot spawn subagents or other blueprints. |
| Resource exhaustion | `max_turns` limit on blueprint. Same 300s timeout as subagents. |

## Concrete Example: GitHubScout

First blueprint implementation. Capability: `github_scout`.

### GitHubScout Capability

```rust
pub struct GitHubScoutCapability;

impl Capability for GitHubScoutCapability {
    fn id(&self) -> &str { "github_scout" }
    fn name(&self) -> &str { "GitHub Scout" }
    fn description(&self) -> &str {
        "Pre-built agent that searches GitHub repositories for code, issues, and discussions."
    }
    fn status(&self) -> CapabilityStatus { CapabilityStatus::Available }
    fn icon(&self) -> Option<&str> { Some("github") }
    fn category(&self) -> Option<&str> { Some("Orchestration") }

    // No tools for host — all tools are private to the blueprint
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
                    },
                    "max_results": {
                        "type": "integer",
                        "default": 20,
                        "description": "Maximum results per search query"
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

These tools use the GitHub API token from the capability's configuration (via User Connections or environment). They are never visible to the host agent.

### Usage

Host agent delegates GitHub research:

```json
{
  "name": "spawn_subagent",
  "arguments": {
    "name": "Scout",
    "task": "Find how authentication middleware is implemented in the fastify repo. Look for JWT validation patterns and report the key files.",
    "blueprint": "github_scout",
    "config": {
      "repos": ["fastify/fastify", "fastify/fastify-jwt"]
    }
  }
}
```

Result: A child session runs with Haiku, the 3 GitHub tools, and the scout prompt. Host gets back a summary. Host never sees `search_github_code` in its own tool list.

## RuntimeAgent Assembly

When spawning from a blueprint, the `RuntimeAgentBuilder` takes a different path:

```
Standard subagent:
  RuntimeAgentBuilder::new()
    .with_harness(parent_harness, ...)     // inherit
    .with_agent(parent_agent, ...)         // inherit
    .build()

Blueprint subagent:
  RuntimeAgentBuilder::new()
    .system_prompt(blueprint.system_prompt)  // blueprint's own prompt
    .tools(blueprint.tools)                  // blueprint's private tools
    .model(resolve_model(blueprint.model))   // blueprint's model strategy
    .max_iterations(blueprint.max_turns)
    .build()
```

The blueprint path skips harness/agent inheritance entirely. The blueprint is self-contained.

## Relationship to Subagents

| Aspect | Subagent | Blueprint |
|--------|----------|-----------|
| Prompt | Controlled by host (task = prompt) | Baked in by blueprint author |
| Tools | Inherits parent's tools | Private tools, no inheritance |
| Model | Inherits parent's model | Fixed/Default/Inherit per blueprint |
| Config | None (host controls everything via task) | Narrow, typed, validated |
| Use case | Offload work the host knows how to do | Delegate to specialist the host doesn't need to understand |
| Invocation | `spawn_subagent(name, task)` | `spawn_subagent(name, task, blueprint, config)` |
| Tool visibility | Host's tools visible in child | Blueprint's tools invisible to host |
| Example | "Run these 5 tests" | "Search GitHub for auth patterns" |

Both use the same 3-tool contract (`spawn/get/message`). Both create child sessions with `parent_session_id`. Both respect nesting prevention. The difference is purely in RuntimeAgent assembly.

## Implementation Path

### Phase 1: Core Infrastructure

1. Add `AgentBlueprint` and `BlueprintModel` structs to `crates/core/src/capabilities/mod.rs`
2. Add `agent_blueprints()` default method to `Capability` trait
3. Add `blueprints()` and `blueprint(id)` to `CapabilityRegistry`
4. Extend `spawn_subagent` tool schema with `blueprint` and `config` parameters
5. Extend `SpawnSubagentTool::execute_with_context` to handle blueprint path
6. Add blueprint discovery system prompt contribution

### Phase 2: GitHubScout

1. New integration crate: `integrations/github-scout/`
2. Implement `GitHubScoutCapability` with 3 private tools
3. Wire GitHub API via User Connections (OAuth token)
4. Register via `inventory::submit!` plugin system
5. Add to Generic harness capabilities

### Phase 3: Framework Maturation

| Feature | Description |
|---------|-------------|
| Blueprint config injection | Pass validated config into tool context or prompt template |
| Blueprint-scoped MCP | Blueprint can declare MCP servers that start/stop with the child session |
| Blueprint isolation modes | `shared_ro` / `isolated` filesystem access |
| More blueprints | CodeReviewer, TestRunner, DocWriter as built-in blueprints |

## Open Questions

1. **Should blueprints have access to parent's filesystem?** Current lean: no by default. GitHubScout doesn't need it. Future blueprints (CodeReviewer) might want read-only access — add as opt-in field on `AgentBlueprint`.

2. **Should `get_subagents` distinguish blueprint-spawned children?** Probably yes — include `blueprint_id` in the response so the host knows which specialist it's talking to.

3. **Can a capability contribute both host tools AND a blueprint?** Yes. Example: a `github` capability could give the host `create_github_issue` directly AND contribute a GitHubScout blueprint for research. The host tool and the blueprint tools are separate namespaces.
