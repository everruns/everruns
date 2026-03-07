# Tool Search Specification

## Abstract

Tool search enables deferred tool loading — instead of sending all tool definitions (with full JSON schemas) to the LLM on every request, only tool names and descriptions are sent upfront. Full schemas are loaded on-demand when the model decides it needs a tool. This reduces token usage, cost, and latency for agents with many capabilities.

Inspired by OpenAI's GPT-5.4 `tool_search` feature, but designed as a provider-agnostic capability within the everruns architecture.

## Motivation

Current flow: all capability tools → `RuntimeAgent.tools` → `LlmCallConfig.tools` → full JSON schemas sent to every LLM call. With 30+ capabilities and MCP servers, tool definitions can consume 5K-15K tokens per request.

GPT-5.4's `tool_search` demonstrated 47% token reduction on 250-tool workloads at same accuracy. We want similar benefits, ideally across all providers.

## How GPT-5.4 tool_search Works

OpenAI's implementation has two modes:

### Hosted Mode

```json
{
  "model": "gpt-5.4",
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

## Design: Capability-Driven Tool Search

### Decision: Model Profile Flag

Add `tool_search: bool` to `LlmModelProfile`. Only models that natively support `tool_search` (currently GPT-5.4+) get the optimized path. Other models continue receiving full tool definitions.

```rust
pub struct LlmModelProfile {
    // ... existing fields ...
    /// Whether the model supports tool_search (deferred tool loading)
    pub tool_search: bool,
}
```

This is the right granularity — tool_search is a model-level capability like `tool_call`, `reasoning`, or `structured_output`. The profile already drives behavior in `ReasonAtom` (e.g., stripping reasoning_effort for non-reasoning models).

### Decision: Hosted Mode Only (Initially)

Use hosted tool_search (server-side, single-turn). Client-executed mode adds multi-turn complexity for marginal benefit — OpenAI's hosted search already handles the matching. Client-executed mode is a future option if we need custom tool registries or cross-provider search.

### Decision: Capability Categories as Namespaces

Map capability categories to tool_search namespaces. Each capability already has `category` (Data, Management, MCP Servers, etc.) and a description. This maps cleanly:

```
Capability(session_file_system, category="File System")
  → Namespace("file_system", "File system tools for reading, writing, and managing files")
    → Functions: read_file, write_file, list_directory, ... (all defer_loading: true)
```

For MCP servers, each server is already a natural namespace:

```
MCP Server("github", "GitHub integration")
  → Namespace("mcp_github", "GitHub integration for repos, issues, PRs")
    → Functions: mcp_github__search_repos, mcp_github__create_issue, ...
```

### Decision: Model Gate in RuntimeAgentBuilder, Deferral in Driver

The model compatibility check lives in `RuntimeAgentBuilder::build()` — if the model doesn't support tool_search (per `LlmModelProfile`), `tool_search` is cleared before reaching the driver. This prevents unsupported API parameters from ever reaching the LLM provider.

The deferral logic (namespace grouping, defer_loading) lives in the OpenAI driver's `convert_tools()`. Capabilities don't need to know about tool_search — they just provide tools as today.

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
              LlmCallConfig.tools    LlmDriver restructures:
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

- `write_todos` — used for task tracking on most turns
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

### GPT-5.4 Family

```rust
"gpt-5.4" => Some(LlmModelProfile {
    // ... existing fields ...
    tool_search: true,  // NEW
}),

"gpt-5.4-pro" => Some(LlmModelProfile {
    // ... existing fields ...
    tool_search: true,  // NEW
}),
```

### All Other Models

```rust
tool_search: false,  // Default for all existing models
```

When other providers adopt similar features (Anthropic, Gemini), flip the flag per model.

## OpenAI Driver Changes

### ResponsesTool Enum Extension

```rust
enum ResponsesTool {
    Function {
        r#type: String,
        name: String,
        description: String,
        parameters: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        defer_loading: Option<bool>,
    },
    Namespace {
        r#type: String,
        name: String,
        description: String,
        tools: Vec<ResponsesTool>,
    },
    ToolSearch {
        r#type: String,
    },
}
```

### convert_tools with tool_search

```rust
fn convert_tools_with_search(
    tools: &[ToolDefinition],
    threshold: usize,
) -> Vec<ResponsesTool> {
    if tools.len() < threshold {
        return Self::convert_tools(tools);  // existing path
    }

    let mut namespaces: HashMap<String, Vec<ResponsesTool>> = HashMap::new();
    let mut ungrouped = vec![];

    for tool in tools {
        let func = ResponsesTool::Function {
            r#type: "function".into(),
            name: tool.name().into(),
            description: tool.description().into(),
            parameters: tool.parameters().clone(),
            defer_loading: Some(true),
        };

        match tool.category() {
            Some(cat) => namespaces.entry(cat.into()).or_default().push(func),
            None => ungrouped.push(func),
        }
    }

    let mut result: Vec<ResponsesTool> = namespaces
        .into_iter()
        .map(|(name, tools)| ResponsesTool::Namespace {
            r#type: "namespace".into(),
            name,
            description: format!("Tools for {}", name),
            tools,
        })
        .collect();

    result.extend(ungrouped);
    result.push(ResponsesTool::ToolSearch { r#type: "tool_search".into() });
    result
}
```

## Provider Abstraction

### Non-OpenAI Providers

For Anthropic and Gemini (no native tool_search):

1. **Short-term**: Send full tool definitions (current behavior). `tool_search: false` in profile.
2. **Medium-term**: Implement client-side tool search as a pre-processing step in the driver — send only names + descriptions as a summary tool, handle tool discovery in the agent loop.
3. **Long-term**: When providers adopt similar features, use their native APIs.

### Client-Side Tool Search (Future)

For providers without native tool_search, we could implement it ourselves:

1. Send tool names + descriptions in system prompt (not as tool definitions)
2. Add a synthetic `_discover_tools` tool that the model calls with a query
3. Match query against tool descriptions using simple TF-IDF or embedding similarity
4. Inject matched tool definitions into the next turn

This is a significant effort and should wait until we validate the value of tool_search with OpenAI models first.

## LlmCallConfig Changes

No changes needed. `LlmCallConfig.tools` continues to carry all tool definitions. The deferral decision is purely in the driver layer. The driver receives the profile via the model field and looks up `tool_search` support.

However, the driver needs access to the model profile. Two options:

1. **Pass profile through LlmCallConfig** — add `model_profile: Option<LlmModelProfile>` field
2. **Look up in driver** — driver already knows its provider type, can call `get_model_profile()`

Option 2 is cleaner. The driver constructor already has the provider context.

## Observability

Track tool_search effectiveness via existing metadata:

- `tool_search_active: bool` in `LlmCompletionMetadata`
- Compare `prompt_tokens` between tool_search and non-tool_search requests
- Log which tools were actually loaded (from response events, if OpenAI exposes this)

## Implementation Plan

### Phase 1: Model Profile + Driver (Low Risk)

1. Add `tool_search: bool` to `LlmModelProfile` (default false)
2. Set `tool_search: true` for GPT-5.4 family
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

### Phase 3: Cross-Provider (Future)

12. Client-side tool search for Anthropic/Gemini
13. Namespace discovery for MCP servers
14. Per-agent tool_search configuration override

## Open Questions

1. **Should tool_search be configurable per-agent?** Could add `tool_search: Option<bool>` to agent config, overriding model profile default. Useful for agents that always need all tools loaded (e.g., short deterministic workflows).

2. **How does tool_search interact with infinity context?** If we're already compacting context, deferred tools add complexity to the compaction logic. The compacted state may not include tool search results.

3. **MCP server tool discovery latency.** MCP tools are already lazily cached (24h TTL). With tool_search, the model might request tools from an MCP server that hasn't been polled recently. Need to handle stale cache gracefully.

## References

- [OpenAI Tool Search Guide](https://developers.openai.com/api/docs/guides/tools-tool-search/)
- [GPT-5.4 Announcement](https://openai.com/index/introducing-gpt-5-4/)
- [OpenAI Responses API](https://platform.openai.com/docs/api-reference/responses)
- Internal: `specs/capabilities.md`, `specs/llm-drivers.md`
