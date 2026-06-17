# Tool narration

Every tool call shown in a transcript or timeline carries a human-readable
narration line ("Read AGENTS.md", "Searched tools: router"). everruns authors
this narration in the backend so that **downstream clients do not need
per-tool narration code**. A client should be able to render with nothing more
than:

```rust
data.narration.unwrap_or(display_name_or_tool_name)
```

The narration system lives in [`crates/core/src/tool_narration.rs`](../crates/core/src/tool_narration.rs)
and is emitted on tool events (see [`specs/events.md`](events.md) — the
`narration` field). Narration is backend-authored and localizable (see
[`specs/localization.md`](localization.md), `tool.narration.*` keys).

This spec captures *where narration logic lives* and *the rules it must obey*.
The concrete phrasing for each tool is implemented in code, not enumerated
here — read `tool_narration.rs` (and per-capability hooks) for exact strings.

## Where narration logic lives

Narration is computed from a [`ToolCall`](../crates/core/src/tool_types.rs) plus
an optional [`ToolDefinition`](../crates/core/src/tool_types.rs) — both plain,
serializable data. It is rendered in more than one place (the act atom, and
event construction in `events.rs`) and the resulting string is stored on the
event. This rules out hanging narration off the live `Tool` trait: the trait
object is not available everywhere narration is produced, and the most common
tools to narrate (host tools like a CLI's `tool_search`/`activate_skill`, and
arbitrary MCP tools) have no in-process `Tool` impl at all.

So narration has two homes, in priority order:

1. **Capability override** — a capability owns bespoke narration for *its own*
   tools by implementing [`ToolCallHook::narration`](../crates/core/src/capabilities/mod.rs)
   and exposing it via `Capability::tool_call_hooks`. The act atom tries every
   registered hook before falling back to the central engine. This is the right
   home for integration-specific phrasing — e.g. the `browserless_*` tools
   narrate from a `BrowserlessNarrationHook` in the browserless integration, and
   `cursor_*` from a hook in the cursor integration.
2. **Central engine** — `tool_narration.rs` owns generic, name/shape-based rules
   for conventional and external tool names (the `*_search`, `web_fetch`,
   `bash`, `tool_search`, skill/memory/config/hook/connector/background families,
   …) plus the core built-in file tools. This is *required* and cannot be
   pushed into capabilities, because it serves tools everruns does not define.

Tools themselves contribute only the `narration_noun` data hint (see below);
they hold no narration rendering logic.

### Rule of thumb

- Phrasing for a tool family that any host/MCP server might expose → central
  engine (generic or exact rule), so unseen siblings narrate for free.
- Phrasing specific to one everruns capability/integration → that capability's
  `ToolCallHook::narration`.
- A multi-operation CRUD tool you own → set `narration_noun` (no custom code).

## Resolution order

For a tool call and [`ToolNarrationPhase`](../crates/core/src/tool_narration.rs)
(`Started`, `Waiting`, `Completed`, `Failed`):

1. **Capability `ToolCallHook::narration`** — first hook returning `Some` wins.
2. **Central exact-name rule** — known built-in/common tool names.
3. **Central generic pattern rule** — name-shape and argument-shape rules that
   cover whole families (`*_search`, names containing `fetch`/`symbol`/`config`,
   …), catching tools never seen by name.
4. **`narration_noun` operation narration** — CRUD tools whose `ToolHints` set
   `narration_noun` and whose args carry `operation`/`action` (see
   [`specs/tool-execution.md`](tool-execution.md#narration-formatting)).
5. **Generic fallback** — `"{verb} {display_name}"` from the localized display
   name or title-cased tool name.

Earlier steps win. A generic rule must never shadow a more specific exact rule
or capability hook; the generic rules exist only so *un-named* tools still
narrate sensibly.

## Phrasing

Neutral imperative for in-progress phases, neutral past for completed, and
"Could not …" for failed. No first-person ("I'm …"). Prefer a human label from
an argument over a tool-internal name when one is available.

| Phase | Style | Example |
|-------|-------|---------|
| `Started` / `Waiting` | imperative | `Search tools: router` |
| `Completed` | past | `Searched tools: router` |
| `Failed` | could-not | `Could not search tools: router` |

When the relevant argument is absent, drop the value and keep the bare verb
phrase (`Search tools`, `Searched tools`, `Could not search tools`).

## Argument display and redaction

Arguments shown in narration are **display values, not faithful echoes**:

- **Truncate.** queries / patterns / commands: ~48–80 chars, with an ellipsis
  beyond that. File paths show the basename only.
- **URLs.** show host + path; strip the scheme, query string, and fragment (the
  query may carry tokens). Truncate long paths.
- **Never show secrets.** A field whose key contains `token`, `api_key`,
  `password`, `secret`, `authorization`, or similar is never rendered, even when
  it is the only candidate — fall back to the bare verb phrase. The central
  engine enforces this in `safe_arg_str`; capability hooks must uphold it too.
- **Never dump prompts.** Long free text — background-agent instructions, full
  approval-action text, slash-command argument bodies — is not shown; narrate
  the action, not its payload.

## Localization

Narration is subject to backend localization
([`specs/localization.md`](localization.md)); English is the reference. Locales
that have not localized a given family fall back to a generic localized verb
phrase rather than mixing languages (see the Ukrainian path in
`tool_narration.rs`).

## Adding narration for a new tool

1. If the family has a recognizable name/argument shape that other hosts/MCP
   servers could share, add a **generic or exact rule in the central engine** —
   it covers siblings for free.
2. If the phrasing is specific to one capability/integration, implement
   `ToolCallHook::narration` in that capability (return `None` for tools it does
   not own so the central engine still handles them).
3. For a CRUD tool you own, set `narration_noun` in `ToolHints` instead of
   writing rendering code.
4. Pick the display argument carefully — apply the truncation and redaction
   rules above; never select a secret-bearing field.
5. Add unit tests asserting `Started`, `Completed`, and `Failed`, including the
   no-argument fallback (central rules: in `tool_narration.rs`; capability
   hooks: in the owning crate).
