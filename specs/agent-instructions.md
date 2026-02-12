# Agent Instructions (AGENTS.md) Specification

## Abstract

The Agent Instructions capability reads an `AGENTS.md` file from the session workspace and dynamically injects its content into the system prompt on every LLM turn. This provides project-level context and coding conventions to agents without modifying the agent's system prompt directly.

## Background

`AGENTS.md` is an emerging open standard (backed by OpenAI, Google, Cursor, Sourcegraph, Linux Foundation) for providing project-level instructions to AI coding agents. There is no formal specification beyond "plain markdown in a file named AGENTS.md." Each tool defines its own discovery and injection semantics.

Everruns implements AGENTS.md as a built-in capability that reads from the session workspace filesystem. This integrates naturally with the existing capability system—users enable/disable it per agent, and the content is picked up dynamically (no restart needed).

## Design Decisions

| Question | Decision |
|----------|----------|
| File name | `AGENTS.md` only (not CLAUDE.md, .cursorrules, etc.) |
| Discovery | Single file at workspace root (`/AGENTS.md`) — no upward walk (session filesystem is flat) |
| Injection point | Prepended to system prompt, before capability additions |
| Dynamic reading | Re-read on every LLM turn (picks up changes immediately) |
| Size limit | 32 KiB max (truncated with warning if exceeded, matching Codex convention) |
| Missing file | Silently ignored (no error) |
| Format | Plain markdown, no special syntax, no `@` imports |
| Architecture | Capability (marker) + ReasonAtom integration (dynamic reader) |
| Dependencies | None required; `session_file_system` recommended for authoring |

## Capability Definition

```rust
pub struct AgentInstructionsCapability;

impl Capability for AgentInstructionsCapability {
    fn id(&self) -> &str { "agent_instructions" }
    fn name(&self) -> &str { "Agent Instructions" }
    fn description(&self) -> &str {
        "Reads AGENTS.md from the session workspace and includes it as context in the system prompt. Content is re-read on every turn, so changes are picked up automatically."
    }
    fn status(&self) -> CapabilityStatus { CapabilityStatus::Available }
    fn icon(&self) -> Option<&str> { Some("file-text") }
    fn category(&self) -> Option<&str> { Some("Configuration") }
    // No system_prompt_addition() — content is dynamic (read at runtime)
    // No tools
    // No dependencies
}
```

## Integration Flow

```
execute_llm_call()
  ├── Load agent + session
  ├── Resolve capabilities
  ├── Check: is "agent_instructions" in resolved capabilities?
  │   └── Yes: read /AGENTS.md from session file store
  │       ├── File exists: prepend content to system prompt
  │       ├── File missing: skip silently
  │       └── File > 32 KiB: truncate + log warning
  ├── Build RuntimeAgent (capabilities + tools + model)
  └── Execute LLM call
```

### System Prompt Order

After injection, system prompt order (top to bottom):

1. **AGENTS.md content** (project instructions)
2. **Capability system prompt additions** (tool guidance, etc.)
3. **Agent's base system prompt** (agent-specific behavior)

This ensures project-level context comes first, then capability-specific instructions, then the agent's own personality/role.

## ReasonAtom Changes

Add optional `SessionFileStore` to ReasonAtom:

```rust
pub struct ReasonAtom<A, S, M, P, E> {
    // ... existing fields ...
    file_store: Option<Arc<dyn SessionFileStore>>,
}
```

With builder method:

```rust
pub fn with_file_store(mut self, file_store: Arc<dyn SessionFileStore>) -> Self {
    self.file_store = Some(file_store);
    self
}
```

## Constants

```rust
/// Maximum size of AGENTS.md content (32 KiB)
pub const MAX_AGENTS_MD_SIZE: usize = 32_768;

/// File path in the session filesystem
pub const AGENTS_MD_PATH: &str = "/AGENTS.md";

/// Capability ID
pub const AGENT_INSTRUCTIONS_CAPABILITY_ID: &str = "agent_instructions";
```

## API

The capability appears in standard capability endpoints:

```http
GET /v1/capabilities

Response includes:
{
  "id": "agent_instructions",
  "name": "Agent Instructions",
  "description": "Reads AGENTS.md from the session workspace...",
  "status": "available",
  "icon": "file-text",
  "category": "Configuration"
}
```

Enable on an agent:

```http
PATCH /v1/agents/{id}
{ "capabilities": [{ "ref": "agent_instructions" }, ...] }
```

## Usage

1. Enable the `agent_instructions` capability on an agent
2. Write an `AGENTS.md` file to the session workspace (via file tools, bash, or API)
3. The agent automatically reads it on every turn
4. Edit the file anytime — changes take effect on the next turn
