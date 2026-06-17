# Tool narration

Every tool call shown in a transcript or timeline carries a human-readable
narration line ("Read AGENTS.md", "Searched tools: router"). everruns authors
this narration in the backend so that downstream clients can render with
nothing more than:

```rust
data.narration.unwrap_or(display_name_or_tool_name)
```

Narration is emitted on tool events (see [`specs/events.md`](events.md) — the
`narration` field) and is localizable (see [`specs/localization.md`](localization.md),
`tool.narration.*`).

## Narration is owned by the capability that owns the tool

There is **no central, name-keyed narrator** that recognizes tools globally.
Narration is part of the capability contract: a capability narrates the tools
it contributes, and nothing else. This holds for host-registered capabilities
too — when a host (e.g. Yolop) adds a capability with its own tools to the
runtime, that capability narrates them. everruns does not narrate tools it does
not own; unowned/foreign calls fall back to generic display-name phrasing.

The contract is [`Capability::narrate`](../crates/core/src/capabilities/mod.rs):

```rust
fn narrate(
    &self,
    tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> Option<String> { None }  // default: contribute nothing
```

An implementation returns `Some(line)` for its own tool names and `None` for
everything else, so other capabilities — or the generic fallback — can handle
the rest. Implementations should match only the tool names the capability
actually provides.

### Wiring

During capability assembly, the framework wraps each applied capability in a
`CapabilityNarrationHook` (an adapter over the existing
[`ToolCallHook`](../crates/core/src/capabilities/mod.rs) channel) and registers
it on the act atom. These adapters are appended **after** every explicit
tool-call hook, so model-authored narration (the `human_intent` capability)
keeps precedence. The first hook to return `Some` wins; if none do, the act
atom falls back to [`render_tool_narration_with_locale`](../crates/core/src/tool_narration.rs).

## Reusable phrasing helpers

So wording and localization stay consistent without a global registry,
[`crate::tool_narration`](../crates/core/src/tool_narration.rs) exposes
locale-aware phrasing helpers that capabilities call from `narrate()`:
`narrate_read_file`, `narrate_shell_exec`, `narrate_search_web`,
`narrate_web_fetch`, `narrate_tool_search`, `narrate_skill`,
`narrate_secret_store`, `narrate_spawn_subagent`, `narrate_write_todos`, and the
generic builders `generic_phrase` / `labeled_phrase`, plus argument utilities
(`arg_str`, `safe_arg_str`, `basename`, `truncate`, `url_display`).

These are a vocabulary, not a dispatcher: a capability decides *which* helper
applies to *its* tool. Multiple capabilities (e.g. the exec integrations) reuse
the same helper for their respective tools.

## Generic fallback

When no capability narrates a call, `render_tool_narration_with_locale` applies,
in order:

1. **`narration_noun` operation narration** — CRUD tools whose `ToolHints` set
   `narration_noun` and whose args carry `operation`/`action` (see
   [`specs/tool-execution.md`](tool-execution.md#narration-formatting)).
2. **Display-name fallback** — `"{verb} {display_name}"` from the localized
   display name or title-cased tool name.

This is the only path that runs without a capability, and it never matches
specific tool names.

## Phrasing

Neutral imperative for in-progress phases, neutral past for completed, and
"Could not …" for failed. No first-person ("I'm …"). Prefer a human label from
an argument over a tool-internal name when one is available.

| Phase | Style | Example |
|-------|-------|---------|
| `Started` / `Waiting` | imperative | `Search tools: router` |
| `Completed` | past | `Searched tools: router` |
| `Failed` | could-not | `Could not search tools: router` |

When the relevant argument is absent, drop the value and keep the bare verb.

## Argument display and redaction

Arguments shown in narration are **display values, not faithful echoes**:

- **Truncate.** queries / patterns / commands: ~48–80 chars with an ellipsis.
  File paths show the basename only.
- **URLs.** show host + path; strip the scheme, query string, and fragment (the
  query may carry tokens). `url_display` does this.
- **Never show secrets.** A field whose key contains `token`, `api_key`,
  `password`, `secret`, `authorization`, or similar is never rendered, even as
  the only candidate — fall back to the bare verb. `safe_arg_str` enforces this;
  capability `narrate()` implementations must uphold it.
- **Never dump prompts.** Long free text — background-agent instructions, full
  approval-action text, slash-command argument bodies — is not shown.

## Localization

Phrasing helpers carry English and, where localized, Ukrainian wording, keyed by
`locale`. English is the reference; locales without a localized family fall back
to a generic localized verb phrase rather than mixing languages.

## Adding narration for a new tool

1. Implement (or extend) `narrate()` in the **capability that contributes the
   tool**. Match only that capability's tool names; return `None` otherwise.
2. Call an existing phrasing helper in `crate::tool_narration` where one fits;
   add a new helper there only if the wording/localization is reusable.
3. Apply the truncation and redaction rules; never select a secret-bearing
   field.
4. Add a unit test in the owning crate asserting `Started`, `Completed`, and
   `Failed`, including the no-argument fallback, and that the capability returns
   `None` for tools it does not own.
