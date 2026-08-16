---
type: Specification
title: "Tool Search Specification"
description: "OpenAI tool_search deferred tool loading capability."
tags:
  - everruns
  - execution
---
# Tool Search Specification

## Abstract

Tool search enables deferred tool loading, instead of sending all tool definitions (with full JSON schemas) to the LLM on every request, only tool names and descriptions are sent upfront. Full schemas are loaded on-demand when the model decides it needs a tool. This reduces token usage, cost, and latency for agents with many capabilities.

Inspired by OpenAI's `tool_search` feature, but designed as a provider-agnostic capability within the everruns architecture.

### Layering relative to resource discovery

`tool_search` operates over tools that are **already attached** to the agent, it only defers their schemas. The question of *which* MCP server or A2A agent should even be attached in the first place is one layer up, answered by the ARD client capability (`resource_discovery`, [`crates/ard/SPEC.md`](../../crates/ard/SPEC.md)). When ARD attaches an MCP server mid-session, its tools appear next turn as `mcp_<name>__*` and are then subject to this `tool_search` deferral like any other tool.

## Motivation

Current flow: all capability tools -> `RuntimeAgent.tools` -> `LlmCallConfig.tools` -> full JSON schemas sent to every LLM call. With 30+ capabilities and MCP servers, tool definitions can consume 5K-15K tokens per request.

Native tool_search demonstrated ~47% token reduction on large tool sets at same accuracy. We want similar benefits, ideally across all providers.

## How OpenAI tool_search Works

OpenAI's implementation has two modes:

### Hosted Mode

```json
{
  "model": "gpt-5.5",
  "tools": [
    {
      "type": "namespace",
      "name": "crm",
      "description": "CRM tools for customer lookup and order management.",
      "tools": [
        {
          "type": "function",
          "name": "list_open_orders",
          "description": "List open orders for a customer ID.",
          "defer_loading": true,
          "parameters": { "type": "object", ... }
        }
      ]
    },
    { "type": "tool_search" }
  ]
}
```

Model sees name + description upfront, parameters loaded only when needed. For namespaces, model sees only namespace name + description until it searches within.

### Client-Executed Mode

Model emits `tool_search_call` → client responds with `tool_search_output` containing matched tools. Multi-turn handshake.

## Four Capabilities

There are four capabilities, two mechanisms (hosted vs client-side). The two hosted capabilities produce the *same* provider-agnostic `ToolSearchConfig`; the driver that handles the request renders the provider-specific wire format.

- **`openai_tool_search`**: uses OpenAI's hosted tool_search (namespaces + `defer_loading` + `{"type":"tool_search"}`). Gated on the OpenAI `LlmModelProfile.tool_search`. Hosted mode only. See `crates/builtins/src/openai_tool_search.rs`.
- **`claude_tool_search`**: uses Anthropic's hosted tool_search: each deferrable tool gets `defer_loading: true` and a `tool_search_tool_bm25_20251119` server-tool entry is added (no namespaces, Anthropic defers each tool individually). Gated on the Anthropic `LlmModelProfile.tool_search`. The Anthropic driver renders it; see `crates/anthropic/src/driver.rs::convert_tools_with_search` and `crates/builtins/src/claude_tool_search.rs`.
- **`tool_search`**: generic, provider-agnostic client-side deferral that works with any model (Gemini, OpenAI Completions, Claude/GPT reached via a gateway that masks the hosted format, ...). See `crates/builtins/src/tool_search.rs` and below.
- **`auto_tool_search`**: model-adaptive: picks the matching hosted mechanism on models that support it and the generic client-side mechanism everywhere else. This is the right default for a multi-provider harness (it is what the `generic` harness uses). See `crates/builtins/src/auto_tool_search.rs`.

### Auto resolution

`auto_tool_search` is a runtime dispatcher, not a separate mechanism. `AutoToolSearchCapability` owns an `OpenAiToolSearchCapability`, a `ClaudeToolSearchCapability`, and a `ToolSearchCapability`, and implements `Capability::resolve_for_model`. The agent's model is carried on `SystemPromptContext::model`, so capability collection knows it and delegates to exactly one inner capability, collecting *its* contributions in place of the dispatcher's:

- **Model has native OpenAI support** (`openai_tool_search::model_supports_native_tool_search`, GPT-5.4+): resolves to `openai_tool_search`.
- **Model has native Anthropic support** (`claude_tool_search::model_supports_native_tool_search`, Claude Sonnet 4 / Opus 4 / Haiku 4.5 / Fable 5 and newer): resolves to `claude_tool_search`.
- Either hosted branch makes collection set the hosted `ToolSearchConfig` (keyed on the resolved `effective.id()`, which is one of the two hosted ids); no client-side tool or hook is contributed.
- **Model lacks native support, or model is unknown:** resolves to the generic `tool_search`. Collection contributes its `DeferSchemaHook` + `tool_search` tool + system-prompt note and sets no hosted config.

A model id resolves under at most one of the OpenAI/Anthropic profiles, so the order between the two hosted checks is immaterial.

Because the mechanism is chosen during collection, `RuntimeAgentBuilder::build()` does no `auto`-specific pruning. There is no `auto` flag on `ToolSearchConfig`: a hosted config present after collection always means "use hosted deferral" (build only clears it if a *directly* configured `openai_tool_search` / `claude_tool_search` lands on a model that can't honor it, see below). `build()` reconciles by checking the model against **both** hosted providers, `get_model_profile(Openai, …)` and `get_model_profile(Anthropic, …)`, and clears the config only when neither advertises `tool_search`.

The executor (act) path collects without a model and so resolves to the generic mechanism. This registers the `tool_search` tool in the worker registry as a harmless superset: on native models the reason path never shows that tool to the model, so it is never called.

### Generic (client-side) tool_search

Implemented entirely in core, with no driver or agent-loop changes:

1. A `ToolDefinitionHook` (`DeferSchemaHook`) runs in `RuntimeAgentBuilder::build()`. When the agent carries `>= threshold` tools, it replaces each deferrable tool's parameter schema with a minimal open-object stub (name + description survive). The capability prompt carries the `tool_search` instruction once rather than repeating it in every schema. `DeferrablePolicy::Never` tools, tools in the capability's never-defer allowlist, and the `tool_search` tool keep full schemas.
2. A real `tool_search` tool is registered. On call it reads sibling tool schemas from `ToolContext::tool_registry` (the same mechanism `spawn_background` uses) and returns the full schemas of tools matching the query.
3. A system-prompt note tells the model to call `tool_search` before using a tool whose parameters it has not yet loaded.

Because the underlying tools stay registered and executable, tool calls and results are unchanged, only how schemas reach the model differs. The search reads schemas from the worker-side registry, so it covers built-in tools and MCP tools. Three interactions matter:

- **Progressive disclosure (registered schema, not just text).** Structured tool calling makes the model emit arguments against a tool's *registered* schema. Returning the real schema only in the `tool_search` *result* is not enough: the registered schema would stay the stub, so the model could read the schema but have nothing to emit arguments against. To close that, a shared revealed-set (`Arc<Mutex<HashSet<String>>>`) is threaded through the capability, its `DeferSchemaHook`, and the `tool_search` tool. When `tool_search` matches tools it records them as revealed; because the hook re-runs every reasoning iteration, the next iteration advertises those tools with their full schema on the *registered* definition. The permissive stub (`additionalProperties: true`) remains as a fallback for the first call before a reveal lands. (See `test_revealed_tool_regains_full_schema_next_pass` and `test_search_records_reveal_and_restores_registered_schema`.)
- **Never-defer allowlist (embedder-set policy).** `DeferrablePolicy::Never` is set by a tool's owner. An embedder that composes tools it does not own keeps hot-path tools full by name via `ToolSearchCapability::with_never_defer([...])` or a `never_defer` config array (merged with the programmatic list). Effect is identical to `DeferrablePolicy::Never`, but settable from outside.

- **Mutually exclusive with hosted deferral.** `DeferSchemaHook` opts out of running while a hosted config is present (`ToolDefinitionHook::applies_with_native_tool_search()` → `false`), so the two never both strip schemas. In `RuntimeAgentBuilder::build()`, the presence of a hosted `ToolSearchConfig` makes the hook skip, even on an unsupported model, where the hosted config is then disabled (full schemas, no client-side fallback). So hand-configuring `openai_tool_search` alongside `tool_search` makes the hosted config win regardless of model. This case does not arise with `auto_tool_search`, which resolves to exactly one mechanism at collection time (see "Auto resolution" above) and never sets a hosted config on a non-native model.
- **MCP tools are deferred and searchable.** MCP tool definitions become registry proxies in the act path. `DeferSchemaHook` now strips their schemas like any other deferrable tool, saving the original in `full_parameters` on the definition. When `build_mcp_proxy_tools` builds the proxy it clones the (stripped) `BuiltinTool`, which carries `full_parameters`. When `tool_search` calls `registry.tool_definitions()`, `McpProxyTool::to_definition()` returns this cloned definition, and `d.full_parameters()` returns the real schema.

## Design: Capability-Driven Tool Search

### Decision: Model Profile Flag

Add `tool_search: bool` to `LlmModelProfile`. Only models that natively support `tool_search` get the optimized path. Other models continue receiving full tool definitions.

```rust
pub struct LlmModelProfile {
    // ... existing fields ...
    /// Whether the model supports tool_search (deferred tool loading)
    pub tool_search: bool,
}
```

This is the right granularity, tool_search is a model-level capability like `tool_call`, `reasoning`, or `structured_output`. The profile already drives behavior in `ReasonAtom` (e.g., stripping reasoning_effort for non-reasoning models).

### Decision: Hosted Mode Only (Initially)

Use hosted tool_search (server-side, single-turn). Client-executed mode adds multi-turn complexity for marginal benefit, OpenAI's hosted search already handles the matching. Client-executed mode is a future option if we need custom tool registries or cross-provider search.

### Decision: Capability Categories as Namespaces

Map capability categories to tool_search namespaces. Each capability already has `category` (Data, Management, MCP Servers, etc.) and a description. This maps cleanly:

```
Capability(session_file_system, category="File System")
  → Namespace("file_system", "File system tools for reading, writing, editing, and managing files")
    → Functions: read_file, write_file, edit_file, list_directory, ... (all defer_loading: true)
```

For MCP servers, each server is already a natural namespace:

```
MCP Server("github", "GitHub integration")
  → Namespace("mcp_github", "GitHub integration for repos, issues, PRs")
    → Functions: mcp_github__search_repos, mcp_github__create_issue, ...
```

### Decision: Model Gate in RuntimeAgentBuilder, Deferral in Driver

The model compatibility check lives in `RuntimeAgentBuilder::build()`, if the model doesn't support tool_search (per `LlmModelProfile`), `tool_search` is cleared before reaching the driver. This prevents unsupported API parameters from ever reaching the LLM provider.

The deferral logic (namespace grouping, defer_loading) lives in the OpenAI driver's `convert_tools()`. Capabilities don't need to know about tool_search, they just provide tools as today.

```
Capability.tools()  →  collect_capabilities() sets tool_search config
                               ↓
                      RuntimeAgentBuilder::build()
                      checks model profile.tool_search
                               ↓
                    ┌──────────┴──────────┐
                    │ unsupported model    │ supported model
                    │ (clears tool_search) │ (keeps config)
                    ↓                      ↓
              RuntimeAgent.tool_search=None    RuntimeAgent.tool_search=Some(config)
                    ↓                      ↓
              LlmCallConfig.tools    ChatDriver restructures:
              (full schemas)         - Namespace groupings
                                     - defer_loading: true
                                     - { "type": "tool_search" }
```

### Decision: ToolDefinition Metadata for Grouping

Add optional `category` to `ToolDefinition` so the driver can group tools into namespaces:

```rust
pub struct BuiltinTool {
    pub name: String,
    pub display_name: Option<String>,
    pub description: String,
    pub parameters: serde_json::Value,
    pub policy: ToolPolicy,
    /// Category for tool_search namespace grouping (from parent capability)
    pub category: Option<String>,
}
```

Capabilities already have categories. When `collect_capabilities()` builds the tool list, propagate the capability's category to each tool definition. The driver then groups by category when building namespaces.

## How to Decide What Gets Deferred

### Deferral Heuristic

**All tools are deferred by default** when tool_search is active. The model sees names + descriptions for individual functions, and only namespace-level info for grouped tools. This is the right default because:

1. Tool descriptions are the primary signal for tool selection (not parameter schemas)
2. Parameter schemas are only needed after the model decides to call a tool
3. OpenAI's hosted search handles the matching well

### Exceptions (Never Defer)

Some tools should never be deferred because they're called on nearly every turn:

- `write_todos`, used for task tracking on most turns
- `bash`, core execution path whose exact command schema must remain visible
- Tools in the "always-on" set configured per-agent

This maps to a `defer_loading: bool` field on `ToolDefinition`, defaulting to `true` when tool_search is active. Capabilities can override via `always_loaded_tools() -> Vec<&str>`.

### Tool Count Threshold

Only activate tool_search when `tools.len() >= TOOL_SEARCH_THRESHOLD` (suggested: 15). Below this threshold, full schemas fit comfortably in context and the overhead of deferred loading (potential extra latency on first call) isn't worth it.

```rust
const TOOL_SEARCH_THRESHOLD: usize = 15;

// In driver:
let use_tool_search = profile.tool_search && config.tools.len() >= TOOL_SEARCH_THRESHOLD;
```

## Model Profile Updates

Set `tool_search: true` for models that support it. Default `false` for all others. See `crates/provider/src/model_profiles.rs` for current profile definitions. OpenAI sets the flag per model literal (the `gpt-5.4*` / `gpt-5.5*` families); Anthropic sets it centrally by family in `anthropic_family_supports_tool_search` (Sonnet 4.0+, Opus 4.0+, Haiku 4.5+, Fable 5, per docs.claude.com), since the support rule is a clean family cutoff.

A model's `tool_search: true` flag must be backed by a verified end-to-end round-trip
(deferred-load → schema fetch → tool call) against the live provider, not just the
request-shaping unit tests. The `gpt-5.4*` and `gpt-5.5*` families are covered by
live OpenAI integration tests in `crates/llm-tests/tests/tool_search_test.rs`; the
Claude families are covered by the live Anthropic tests
`test_anthropic_claude_tool_search_low_threshold` and
`test_anthropic_auto_tool_search_resolves_to_hosted` in the same file (run against
Claude Haiku 4.5).

### Provider gating (which transports honor the flag)

There are two enforcement points, and they sit at different layers:

1. **Driver layer (authoritative).** `get_model_profile(provider_type, model)` masks `tool_search` to `false` for every provider type **except** `Openai` and `Anthropic`, the two whose drivers render a hosted format (OpenAI Responses; Anthropic Messages). Each driver looks up the profile for *its own* `provider_type`, so the hosted wire format is only emitted on a transport that implements it:
   - **OpenRouter**: stateless OpenAI-compatible `/responses` shim that accepts but does not implement the hosted tool_search extension. Masked.
   - **Bedrock**: ConverseStream; Anthropic's server-side tool search on Bedrock is only available via the InvokeModel API, which we do not use. Masked.
   - **OpenAI Completions / Gemini**: no hosted tool_search at all. Masked.

2. **Dispatcher layer (`auto_tool_search`).** The dispatcher only sees the **model id** (`SystemPromptContext::model`), *not* the transport, so it cannot consult the transport-masked profile. It checks the bare first-party profiles (`get_model_profile(Openai, …)` / `get_model_profile(Anthropic, …)`). In practice this still routes masked transports to the generic mechanism, because they carry **distinct model ids** (`anthropic.claude-…`, `anthropic/claude-…`, `openai/gpt-…`) that don't resolve to the bare first-party profile.

**Residual edge case.** If a masked transport presents a *bare* first-party id that does resolve (e.g. a `gpt-5.4` served through an OpenAI-compatible gateway), the dispatcher resolves to the hosted capability and sets a hosted config, but the driver (step 1) then suppresses the hosted format for that transport. Because a hosted config is present, the client-side `DeferSchemaHook` is skipped too, so **full schemas are sent with no client-side fallback**. This is a graceful degradation (baseline behavior, no error), not a correctness bug, and it predates Claude support (it applies to `openai_tool_search` + OpenRouter just the same). To force client-side deferral on such a setup, add the generic `tool_search` capability explicitly. Making the dispatcher provider-aware (threading `provider_type` into capability collection) would close the gap but is out of scope here.

## Driver Changes

The OpenAI driver extends `ResponsesTool` with `Namespace` and `ToolSearch` variants and adds `convert_tools_with_search()` (namespace grouping + defer_loading + `{"type":"tool_search"}`). See `crates/provider/src/openresponses_protocol.rs`.

The Anthropic driver adds an `AnthropicToolEntry` (untagged: a function tool or a `tool_search_tool_bm25_20251119` server tool) and `convert_tools_with_search()`, which marks deferrable tools `defer_loading: true` and prepends the search-tool entry. No namespaces, Anthropic defers each tool individually. The hosted search-tool entry is always non-deferred, which also satisfies Anthropic's "at least one tool must be non-deferred" constraint. No beta header is required. See `crates/anthropic/src/driver.rs`. The streaming parser ignores the server-side `server_tool_use` / `tool_search_tool_result` blocks (parse-or-skip) and captures the model's subsequent normal `tool_use` for the discovered tool.

Remaining providers (Gemini, OpenAI Completions) send full tool definitions with `tool_search: false`. When a provider adopts a hosted feature, set the flag and un-mask the provider type.

## LlmCallConfig Changes

No changes needed. `LlmCallConfig.tools` continues to carry all tool definitions. The deferral decision is purely in the driver layer. The driver receives the profile via the model field and looks up `tool_search` support.

However, the driver needs access to the model profile. Two options:

1. **Pass profile through LlmCallConfig**: add `model_profile: Option<LlmModelProfile>` field
2. **Look up in driver**: driver already knows its provider type, can call `get_model_profile()`

Option 2 is cleaner. The driver constructor already has the provider context.

## Observability

Track tool_search effectiveness via existing metadata:

- `tool_search_active: bool` in `LlmCompletionMetadata`
- Compare `prompt_tokens` between tool_search and non-tool_search requests
- Log which tools were actually loaded (from response events, if OpenAI exposes this)

## Implementation Plan

### Phase 1: Model Profile + Driver (Low Risk)

1. Add `tool_search: bool` to `LlmModelProfile` (default false)
2. Set `tool_search: true` for verified models (currently the GPT-5.4 and GPT-5.5 families; see "Model Profile Updates")
3. Add `category: Option<String>` to `BuiltinTool` / `ToolDefinition`
4. Propagate capability category to tool definitions in `collect_capabilities()`
5. Extend `ResponsesTool` enum with `Namespace` and `ToolSearch` variants
6. Add `convert_tools_with_search()` to OpenAI driver
7. Gate on `profile.tool_search && tools.len() >= threshold`

### Phase 2: Tuning + Observability

8. Add `tool_search_active` to completion metadata
9. Compare token usage with/without tool_search in staging
10. Tune `TOOL_SEARCH_THRESHOLD` based on real workloads
11. Identify tools that should never be deferred

## References

- [OpenAI Tool Search Guide](https://developers.openai.com/api/docs/guides/tools-tool-search/)
- [Using GPT-5.5](https://developers.openai.com/api/docs/guides/latest-model)
- [OpenAI Responses API](https://platform.openai.com/docs/api-reference/responses)
- Internal: `knowledge/execution/capabilities.md`, `knowledge/foundations/llm-drivers.md`
