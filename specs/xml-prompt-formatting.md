# XML Prompt Formatting

## Abstract

System prompts use XML tags to create clear boundaries between capability instructions, user-provided project instructions (AGENTS.md), and the agent's base system prompt. This follows Anthropic's recommendation for multi-component prompts and is the cross-provider consensus for structured prompt formatting.

## Rationale

### Problem

When multiple capabilities are enabled, the LLM receives a wall of text where:
- Boundaries between capability sections are ambiguous (only `\n\n` separators)
- User-provided AGENTS.md content bleeds into system capability instructions
- Tool guidance for one capability can be misattributed to another
- The distinction between platform instructions and agent-level behavior is unclear

### Why XML Tags

Anthropic's prompt engineering docs recommend XML tags for multi-component prompts:
> "XML tags can be a game-changer. They help Claude parse your prompts more accurately, leading to higher-quality outputs."

Key advantages over markdown-only formatting:
- **Clear boundaries**: Unambiguous section delimiters reduce misattribution
- **Cross-model support**: XML tags are recommended by Anthropic, Google, and OpenAI
- **Prompt injection resistance**: XML boundaries make it harder for injected content in AGENTS.md to impersonate system instructions
- **Parseability**: Tags enable referencing sections by name (e.g., "the instructions in `<agent-instructions>`")

Trade-offs accepted:
- ~15% more tokens for tags (negligible vs. conversation length)
- Slightly reduced human readability in raw form (mitigated: markdown preserved inside tags)

## Design

### Tag Schema

Three XML tags wrap the three logical sections of the assembled system prompt:

| Tag | Purpose | Content |
|-----|---------|---------|
| `<agent-instructions source="AGENTS.md">` | User-provided project instructions | AGENTS.md file content (markdown) |
| `<capability id="{cap_id}">` | Per-capability tool guidance | Capability's `system_prompt_addition()` (markdown) |
| `<system-prompt>` | Agent's core behavior prompt | Agent's base `system_prompt` field |

### Assembled Prompt Structure

```xml
<agent-instructions source="AGENTS.md">
## Style
Use snake_case for variables.
</agent-instructions>

<capability id="session_file_system">
You have access to file system tools...
</capability>

<capability id="test_math">
You have access to math tools...
</capability>

<system-prompt>
You are a helpful assistant.
</system-prompt>
```

### Ordering

Top to bottom (matches existing order, unchanged):

1. **AGENTS.md** — user-provided project instructions (if `agent_instructions` capability enabled)
2. **Capability prompts** — in capability application order (agent caps, then session caps)
3. **Base system prompt** — agent's core behavior prompt

### Conditional Wrapping

- `<system-prompt>` tags only appear when at least one capability contributes a `system_prompt_addition`. If no capabilities add prompts, the base prompt is used unwrapped.
- `<capability>` tags only appear for capabilities that have a non-None `system_prompt_addition()`.
- `<agent-instructions>` tags only appear when AGENTS.md exists and is non-empty.
- Capabilities without `system_prompt_addition` (e.g., `current_time`, `noop`) add no XML to the prompt.

### Implementation

#### `collect_capabilities_with_configs()` (mod.rs)

Wraps each capability's `system_prompt_addition()` in `<capability id="...">` tags during collection:

```rust
if let Some(addition) = capability.system_prompt_addition() {
    system_prompt_parts.push(format!(
        "<capability id=\"{}\">\n{}\n</capability>",
        cap_id, addition
    ));
}
```

#### `format_agents_md_content()` (agent_instructions.rs)

Wraps AGENTS.md content in `<agent-instructions>` tags:

```rust
let mut result = format!("<agent-instructions source=\"AGENTS.md\">\n{}", body);
// ... truncation handling ...
result.push_str("\n</agent-instructions>");
```

#### `RuntimeAgentBuilder::with_capabilities()` (runtime_agent.rs)

Wraps the base system prompt in `<system-prompt>` tags when capabilities contribute prompts:

```rust
if let Some(prefix) = collected.system_prompt_prefix() {
    if !self.runtime_agent.system_prompt.contains("<system-prompt>") {
        self.runtime_agent.system_prompt = format!(
            "<system-prompt>\n{}\n</system-prompt>",
            self.runtime_agent.system_prompt
        );
    }
    self = self.prepend_system_prompt(prefix);
}
```

Double-wrapping is prevented by checking for existing `<system-prompt>` tag (relevant when session capabilities are applied after agent capabilities).

#### `apply_capabilities()` (mod.rs)

Same wrapping in the standalone `apply_capabilities` path:

```rust
let final_system_prompt = match collected.system_prompt_prefix() {
    Some(prefix) => format!(
        "{}\n\n<system-prompt>\n{}\n</system-prompt>",
        prefix, base_runtime_agent.system_prompt
    ),
    None => base_runtime_agent.system_prompt,
};
```

### UI Impact

- **Agent Preview** (Full System Prompt card): Shows raw assembled prompt including XML tags. Uses monospace `<pre>` rendering — XML tags display naturally.
- **Capability Detail** (System Prompt Addition card): Shows the raw `system_prompt_addition()` via `MarkdownDisplay` — no XML tags (wrapping is at collection time, not in the trait).
- **Prompt Editor**: No change — users edit the base system prompt without XML.

### Future Considerations

- If capability prompts need to reference each other, the `id` attribute on `<capability>` tags enables this.
- Tags could be extended with metadata (e.g., `<capability id="..." version="1.0">`) for capability versioning.
