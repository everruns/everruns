---
type: Specification
title: "Tool narration"
description: "Backend-authored, argument-aware narration for common tool families."
tags:
  - everruns
  - execution
---
# Tool narration

Every tool call shown in a transcript or timeline carries a human-readable
narration line ("Read AGENTS.md", "Searched tools: router"). everruns authors
this narration in the backend so that downstream clients can render with
nothing more than:

```rust
data.narration.unwrap_or(display_name_or_tool_name)
```

Narration is emitted on tool events (see [`knowledge/execution/events.md`](events.md), the
`narration` field) and is localizable (see [`knowledge/operations/localization.md`](../operations/localization.md),
`tool.narration.*`).

## Narration is owned by the tool, aggregated by its capability

There is **no central, name-keyed narrator** that recognizes tools globally.
Narration is owned by the tool that defines it, and surfaced by the capability
that contributes that tool. This holds for host-registered capabilities too,
when a host (e.g. Yolop) adds a capability with its own tools to the runtime,
those tools narrate themselves. everruns does not narrate tools it does not own;
unowned/foreign calls fall back to generic display-name phrasing.

Two levels:

1. **Tool level (default).** [`Tool::narrate`](../../crates/core/src/tools.rs)
   returns the line for a call to that tool (default `None`). The tool knows its
   own arguments, so this is the natural home for its wording.

   ```rust
   fn narrate(&self, call: &ToolCall, phase: ToolNarrationPhase, locale: Option<&str>)
       -> Option<String> { None }
   ```

2. **Capability level (aggregation + override).**
   [`Capability::narrate`](../../crates/core/src/capabilities/mod.rs) **defaults** to
   dispatching to the matching tool's `narrate()`:

   ```rust
   fn narrate(&self, _def, call, phase, locale) -> Option<String> {
       self.tools().iter()
           .find(|t| t.name() == call.name)
           .and_then(|t| t.narrate(call, phase, locale))
   }
   ```

   A capability **overrides** this only when narration is config-driven, spans
   tools, or the tools are dynamic with no local `Tool` struct, e.g. the `mcp`
   capability narrates proxied `*__search` tools by pattern, and the
   `browserless`/`cursor` integrations narrate their families in one place.

So adding a tool that implements `Tool::narrate` "just works", nothing to wire
in the capability. A capability returns `None` for tools it does not provide, so
other capabilities or the generic fallback can handle them.

Ownership boundary (EVE-876): narration is an execution-semantic value, it is
authored during tool execution and persisted on tool events, so it stays with
the tool/capability contracts in `everruns-core`
([`tool_narration`](../../crates/core/src/tool_narration.rs)). Observability
exporters never author or re-format narration; they only export the `narration`
field already carried by events (e.g. the Braintrust listener includes it in
span payloads).

### Wiring

During capability assembly, the framework wraps each applied capability in a
`CapabilityNarrationHook` (an adapter over the existing
[`ToolCallHook`](../../crates/core/src/capabilities/mod.rs) channel) and registers
it on the act atom. These adapters are appended **after** every explicit
tool-call hook, so model-authored narration (the `human_intent` capability)
keeps precedence. The first hook to return `Some` wins; if none do, the act
atom falls back to [`render_tool_narration_with_locale`](../../crates/core/src/tool_narration.rs).

The default `Capability::narrate` constructs `self.tools()` per call to find the
match; narration is low-frequency, but a capability with expensive `tools()` can
override `narrate()` with a direct match.

## Reusable phrasing helpers

So wording and localization stay consistent without a global registry,
[`crate::tool_narration`](../../crates/core/src/tool_narration.rs) exposes
locale-aware phrasing helpers that capabilities call from `narrate()`:
`narrate_read_file`, `narrate_shell_exec`, `narrate_search_web`,
`narrate_web_fetch`, `narrate_tool_search`, `narrate_skill`,
`narrate_secret_store`, `narrate_subagent_spawn`, `narrate_write_todos`, and the
generic builders `generic_phrase` / `labeled_phrase`, plus argument utilities
(`arg_str`, `safe_arg_str`, `basename`, `truncate`, `url_display`).

These are a vocabulary, not a dispatcher: a capability decides *which* helper
applies to *its* tool. Multiple capabilities (e.g. the exec integrations) reuse
the same helper for their respective tools.

## Generic fallback

When no capability narrates a call, `render_tool_narration_with_locale` applies,
in order:

1. **`narration_noun` operation narration**: CRUD tools whose `ToolHints` set
   `narration_noun` and whose args carry `operation`/`action` (see
   [`knowledge/execution/tool-execution.md`](tool-execution.md#narration-formatting)).
2. **Display-name fallback**: `"{verb} {display_name}"` from the localized
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

Grouped headlines use those same bare action phrases. Repeated equivalent actions collapse to a
localized count-aware phrase ("Searched files twice" / "Searched files 3 times") while the child
rows retain their argument-specific narration. Mixed groups preserve the first two distinct action
summaries, then use a localized count of remaining actions so the headline stays bounded.

## Argument display and redaction

Arguments shown in narration are **display values, not faithful echoes**:

- **Filesystem paths.** When a [`SessionFileSystem`] is available on the act path,
  path-bearing helpers (e.g. `narrate_list_directory`) render through
  `display_path()` so narration matches tool results. Capabilities receive this
  via [`ToolNarrationContext`] on `Tool::narrate` / `ToolCallHook::narration`.
  Offline builders without a store fall back to legacy argument echo.
- **Truncate.** queries / patterns / commands: ~48–80 chars with an ellipsis.
  File paths show the basename only.
- **URLs.** show host + path; strip the scheme, query string, and fragment (the
  query may carry tokens). `url_display` does this.
- **Never show secrets.** A field whose key contains `token`, `api_key`,
  `password`, `secret`, `authorization`, or similar is never rendered, even as
  the only candidate, fall back to the bare verb. `safe_arg_str` enforces this;
  capability `narrate()` implementations must uphold it.
- **Never dump prompts.** Long free text, background-agent instructions, full
  approval-action text, slash-command argument bodies, is not shown.

## Localization

Phrasing helpers carry English and, where localized, Ukrainian wording, keyed by
`locale`. English is the reference; locales without a localized family fall back
to a generic localized verb phrase rather than mixing languages.

## Adding narration for a new tool

1. Implement `Tool::narrate` on the **tool struct**, calling an existing phrasing
   helper in `crate::tool_narration` where one fits (add a new helper there only
   if the wording/localization is reusable). The capability surfaces it
   automatically via the default `Capability::narrate`.
2. Override `Capability::narrate` instead only when narration is config-driven,
   spans tools, or the tools are dynamic (no local `Tool` struct).
3. Apply the truncation and redaction rules; never select a secret-bearing
   field.
4. Add a unit test asserting `Started`, `Completed`, and `Failed`, including the
   no-argument fallback.

## Regression guard

`builtin_tools_have_narration_or_documented_generic_fallback` (in
`crates/core/src/capabilities/mod.rs`) walks every tool of every built-in
production capability and fails unless the tool is **covered**: its capability
`narrate()` returns `Some`, or it carries a `narration_noun` hint (data-driven
CRUD narration). A capability whose generic display-name presentation is
deliberate is listed in that test's `GENERIC_NARRATION_ALLOWLIST` with a
documented reason (demo/eval fixtures, operator-only admin surfaces, arbitrary
code execution). A newly added built-in tool that neither narrates nor is
allowlisted trips this test rather than silently falling back to the raw
tool-call presentation.
