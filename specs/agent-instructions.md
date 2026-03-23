# AGENTS.md Specification

## Abstract

The AGENTS.md capability reads an `AGENTS.md` file from the session workspace and dynamically injects its content into the system prompt on every LLM turn. This provides project-level context and coding conventions to agents without modifying the agent's system prompt directly.

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
| Link-following hint | Appended after content; nudges LLM to read referenced files progressively |
| Architecture | Self-contained capability with `system_prompt_contribution()` override |
| Dependencies | None required; `session_file_system` recommended for authoring |

## Capability Definition

The capability encapsulates all AGENTS.md logic: reading from the session filesystem,
formatting, size limiting, and XML wrapping. It uses the `system_prompt_contribution()`
async method (via `SystemPromptContext`) to access the session filesystem.

```rust
pub struct AgentInstructionsCapability;

#[async_trait]
impl Capability for AgentInstructionsCapability {
    fn id(&self) -> &str { "agent_instructions" }
    fn name(&self) -> &str { "AGENTS.md" }
    fn status(&self) -> CapabilityStatus { CapabilityStatus::Available }
    fn icon(&self) -> Option<&str> { Some("file-text") }
    fn category(&self) -> Option<&str> { Some("Configuration") }

    // No static system_prompt_addition — content is dynamic

    async fn system_prompt_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        // Reads /AGENTS.md from ctx.file_store
        // Formats with <agent-instructions> XML wrapping
        // Returns None if file missing or empty
    }
}
```

## SystemPromptContext

Capabilities that need dynamic system prompt content receive a `SystemPromptContext`
with access to session-specific resources:

```rust
pub struct SystemPromptContext {
    pub session_id: SessionId,
    pub file_store: Option<Arc<dyn SessionFileStore>>,
}
```

The context is constructed in `ReasonAtom` and passed through the async builder
methods (`with_harness_async`, `with_agent_async`, `with_capabilities_async`).

## Integration Flow

```
execute_llm_call()
  ├── Load agent + session
  ├── Create SystemPromptContext { session_id, file_store }
  ├── Build RuntimeAgent using async builder methods
  │   ├── with_harness_async() — resolves harness capabilities (including agent_instructions)
  │   ├── with_agent_async() — resolves agent capabilities
  │   └── with_capabilities_async() — resolves session capabilities
  │       └── For each capability: call system_prompt_contribution(ctx)
  │           └── agent_instructions: reads /AGENTS.md from file store
  ├── Build final RuntimeAgent
  └── Execute LLM call
```

### System Prompt Order

After injection, system prompt order (top to bottom):

1. **AGENTS.md content** — wrapped in `<agent-instructions source="AGENTS.md">` tags
2. **Capability system prompt additions** — each wrapped in `<capability id="...">` tags
3. **Agent's base system prompt** — wrapped in `<system-prompt>` tags (only when capabilities are present)

XML tags provide clear boundaries between sections. See `specs/xml-prompt-formatting.md` for rationale.

## ReasonAtom Changes

ReasonAtom holds an optional `SessionFileStore` that is passed to capabilities
via `SystemPromptContext`:

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
  "name": "AGENTS.md",
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
