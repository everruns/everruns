# RLM (Recursive Language Models) Implementation Analysis

## Background

Recursive Language Models (RLMs) are an inference-time paradigm from [Zhang, Kraska & Khattab (arXiv:2512.24601)](https://arxiv.org/abs/2512.24601) that treats long prompts as part of an external REPL environment. Instead of stuffing context into the LLM's attention window, RLMs store context as a variable in a persistent environment and allow the LLM to programmatically examine, decompose, and recursively call sub-LLMs over snippets.

The user referenced [rawwerks/ypi](https://github.com/rawwerks/ypi), a bash-based RLM implementation built on the Pi coding agent. YPI adds a single recursive function (`rlm_query`) to a bash REPL and teaches the LLM to use it via system prompt.

This document analyzes how to implement RLM patterns within Everruns.

## What RLM Actually Is

### Core Concept

```
Standard LLM call:
  llm.completion(query + context)  →  response

RLM call:
  rlm.completion(query, context)   →  response

  Internally:
  1. Context stored as variable in REPL environment
  2. Root LLM receives only the query + awareness that context exists
  3. LLM writes code to inspect/slice/filter context
  4. LLM calls llm_query(sub_prompt, sub_context) for semantic work
  5. LLM combines sub-results programmatically
  6. Returns FINAL(answer) or FINAL_VAR(variable_name)
```

### Key Properties

| Property | Description |
|----------|-------------|
| **Context as variable** | Prompt stored in REPL memory, not attention window |
| **Symbolic recursion** | Sub-calls constructed programmatically (loops, conditionals), not just verbalized |
| **Depth control** | Max recursion depth prevents infinite loops |
| **Cost tracking** | Per-call and cumulative budget enforcement |
| **Workspace isolation** | Each recursive level operates in isolated environment |
| **Model routing** | Child calls can use cheaper/faster models |

### YPI's Implementation (Reference)

YPI is elegantly minimal — three components:

1. **`ypi` launcher** — sets up env vars (`RLM_BUDGET`, `RLM_MAX_DEPTH`, `RLM_CHILD_MODEL`), starts Pi agent
2. **`rlm_query "prompt"`** — spawns child Pi process in isolated jj workspace, returns result
3. **`SYSTEM_PROMPT.md`** — teaches LLM recursive decomposition patterns
4. **`rlm_cost`** — reports cumulative spend/tokens/calls

No HTTP server, no bridge — bash is the execution layer for recursion.

## How RLM Maps to Everruns

Everruns already has most of the building blocks. The question is which integration level to target.

### Architectural Alignment

| RLM Concept | Everruns Equivalent | Gap |
|-------------|---------------------|-----|
| REPL environment | `virtual_bash` capability | Need to inject `llm_query` function |
| Context variable | Session file store / session storage | Need convention for context loading |
| `llm_query()` function | New tool or bash function | **Core gap** — needs implementation |
| Recursion depth | Turn metadata / capability config | Need depth tracking |
| Cost tracking | Usage tracking (existing) | Need per-recursion aggregation |
| Workspace isolation | Session isolation (existing) | Need sub-session creation |
| Model routing | Controls.model_id (existing) | Need child model config |
| System prompt for recursion | Capability system_prompt_addition | Need RLM-specific prompt |

## Implementation Options

### Option A: Capability-Level RLM (Native Tool)

Add `llm_query` as a **new capability** with a built-in tool, running sub-agent turns within Everruns.

```
Agent (depth=0)
  │
  ├── reason: LLM call → tool_call: llm_query("summarize chunk 1", context_slice)
  │     │
  │     └── ActAtom executes llm_query tool
  │           │
  │           ├── Creates sub-turn (depth=1)
  │           ├── Builds RuntimeAgent with reduced max_iterations
  │           ├── Runs ReasonAtom with sub-prompt + context
  │           └── Returns sub-agent response as tool result
  │
  ├── reason: LLM call → tool_call: llm_query("summarize chunk 2", context_slice)
  │     └── ... (parallel or sequential)
  │
  └── reason: LLM combines results → final answer
```

**New files:**
- `crates/core/src/capabilities/llm_query.rs` — capability + tool
- System prompt addition teaching the agent RLM patterns

**Key implementation:**
```rust
// Pseudocode for the llm_query tool
pub struct LlmQueryTool {
    llm_driver: Arc<dyn LlmDriver>,
    max_depth: u32,
    child_model: Option<String>,
    budget_tracker: Arc<BudgetTracker>,
}

impl Tool for LlmQueryTool {
    fn name(&self) -> &str { "llm_query" }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The question or task" },
                "context": { "type": "string", "description": "Context to provide" },
                "model": { "type": "string", "description": "Optional model override" }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> ToolExecutionResult {
        // 1. Check depth limit
        // 2. Check budget
        // 3. Build messages: system prompt + user message with context
        // 4. Call LLM driver
        // 5. Track cost
        // 6. Return response as ToolExecutionResult::Success
    }
}
```

**Pros:**
- Fully integrated with Everruns event system, observability, usage tracking
- Durable execution support — each sub-call is a tracked activity
- Provider-agnostic — works with OpenAI, Anthropic, Gemini
- Parallel sub-calls via ActAtom's `join_all()` pattern
- Cost tracking through existing usage infrastructure

**Cons:**
- More complex than YPI's bash approach
- Each sub-call goes through full Everruns machinery (events, storage)
- Sub-calls lack true REPL state persistence between iterations

### Option B: Virtual Bash Extension (YPI-Style)

Inject `llm_query` as a **bash function** inside the existing `virtual_bash` capability.

```
Agent with virtual_bash
  │
  ├── reason: LLM generates bash code:
  │     context=$(cat /session/files/large_document.txt)
  │     chunk1="${context:0:10000}"
  │     result1=$(llm_query "summarize: $chunk1")
  │     chunk2="${context:10000:10000}"
  │     result2=$(llm_query "summarize: $chunk2")
  │     echo "Combined: $result1 + $result2"
  │
  └── act: virtual_bash executes → llm_query shells out to LLM API
```

**Changes:**
- Extend bashkit sandbox to include `llm_query` command
- Command calls Everruns API or directly invokes LLM driver
- Environment variables for budget/depth/model passed into sandbox

**Pros:**
- Closest to YPI's model — bash is the REPL, context is a variable
- True symbolic recursion — LLM writes loops/conditionals around `llm_query`
- Minimal architectural changes
- Leverages existing bash sandbox isolation

**Cons:**
- Security: exposing LLM API access from bash sandbox requires careful controls
- No durable execution for sub-calls (fire-and-forget from bash)
- Harder to track costs at Everruns level
- Sandbox escaping risk — `llm_query` needs network access

### Option C: Hybrid — Capability with REPL Semantics

Combine both: new `rlm` capability provides both a structured `llm_query` tool AND a REPL-like environment via session storage.

```
Agent with rlm capability
  │
  ├── Tools available:
  │   ├── rlm_query(query, context?)     — invoke sub-LLM
  │   ├── rlm_batch_query(queries[])     — parallel sub-LLM calls
  │   ├── rlm_store(key, value)          — persist intermediate results
  │   ├── rlm_load(key)                  — retrieve stored results
  │   └── rlm_cost()                     — check remaining budget
  │
  ├── reason: LLM plans decomposition strategy
  │   → tool_call: rlm_store("chunks", split_context_into_chunks(context))
  │
  ├── act: stores chunks in session storage
  │
  ├── reason: LLM calls sub-queries
  │   → tool_calls: [
  │       rlm_query("summarize chunk 0", chunk_0),
  │       rlm_query("summarize chunk 1", chunk_1),
  │       ...
  │     ]
  │
  ├── act: parallel sub-LLM calls, results returned
  │
  ├── reason: LLM combines results
  │   → tool_call: rlm_store("final", combined_answer)
  │
  └── turn complete with final answer
```

**Pros:**
- Structured tool calls give full observability and cost tracking
- Parallel sub-calls via existing ActAtom parallel execution
- Session storage provides REPL-like variable persistence
- Budget enforcement at capability level
- Works within existing turn state machine (no new execution model)
- `rlm_batch_query` maps to batch/parallel LLM calls

**Cons:**
- Not true symbolic recursion (LLM can't write `for` loops around `llm_query`)
- Each sub-call is a separate tool call, not programmatic
- More structured but less flexible than bash-based approach

## Recommended Approach: Option C (Hybrid) with Option B Extension

### Phase 1: `rlm` Capability (Option C)

**Rationale:** Fits cleanly into Everruns capability model, provides immediate value, full observability.

#### New Capability: `llm_query`

```
crates/core/src/capabilities/llm_query.rs
```

**Capability ID:** `llm_query`

**Tools provided:**

| Tool | Parameters | Description |
|------|------------|-------------|
| `llm_query` | `query: string, context?: string, model?: string` | Invoke sub-LLM call |
| `llm_batch_query` | `queries: [{query, context}], model?: string` | Parallel sub-LLM calls |
| `llm_cost` | — | Report cumulative sub-call spend |

**Capability config:**

```json
{
  "max_depth": 2,
  "max_calls_per_turn": 50,
  "budget_dollars": 1.0,
  "child_model": "gpt-4.1-mini",
  "timeout_seconds": 30
}
```

**System prompt addition:**
```xml
<llm-query>
You have access to recursive LLM sub-calls via the llm_query tool.

When facing tasks that involve large contexts or complex decomposition:
1. Break the problem into sub-tasks
2. Use llm_query to delegate semantic work to a sub-model
3. Combine results programmatically

Guidelines:
- Each llm_query call receives only the query + context you provide (not your full history)
- Use llm_batch_query for independent sub-tasks (runs in parallel)
- Check llm_cost to stay within budget
- Sub-calls use a smaller model by default — keep sub-queries focused and specific
- Store intermediate results as tool outputs, not in your response

Cost awareness:
- Each sub-call costs tokens. Prefer fewer, well-targeted queries over many small ones.
- Chunk context strategically — overlap boundaries slightly for continuity.
</llm-query>
```

#### Implementation Architecture

```
LlmQueryCapability
  ├── LlmQueryTool
  │   ├── Receives: query + optional context
  │   ├── Builds: [system_message, user_message(query + context)]
  │   ├── Calls: LlmDriver::chat_completion_stream()
  │   ├── Collects: Full response text
  │   ├── Tracks: Token usage, cost, call count
  │   └── Returns: ToolExecutionResult::Success(response)
  │
  ├── LlmBatchQueryTool
  │   ├── Receives: array of {query, context} pairs
  │   ├── Executes: All sub-calls in parallel (tokio::join_all)
  │   ├── Tracks: Aggregate usage
  │   └── Returns: Array of responses
  │
  └── LlmCostTool
      └── Returns: { total_cost, total_tokens, total_calls, budget_remaining }
```

#### Integration Points

1. **LLM Driver Access:**
   - Capability needs `Arc<dyn LlmDriver>` for the child model
   - Resolve via `LlmDriverRegistry` at capability initialization
   - Child model specified in capability config or defaults to agent's model

2. **Cost Tracking:**
   - Extend `ToolContext` with optional `UsageTracker` reference
   - Each sub-call reports `LlmCompletionMetadata` (tokens, model)
   - Aggregate tracked at capability level per turn
   - Budget enforcement: check before each call, fail gracefully if exceeded

3. **Event Emission:**
   - Each `llm_query` call emits events:
     - `llm_query.started` — sub-call initiated
     - `llm_query.completed` — sub-call finished with usage
   - Parent span linkage via `parent_span_id` in event context

4. **Durable Execution:**
   - In production mode, each `llm_query` is a durable activity
   - Retry policy: 2 retries with exponential backoff
   - Timeout: `timeout_seconds` from capability config
   - Failed sub-calls return `ToolError`, not crash

### Phase 2: Virtual Bash Integration (Option B)

Once the capability exists, expose `llm_query` inside virtual_bash:

1. **Bash function injection:**
   ```bash
   llm_query() {
     # Calls Everruns internal API endpoint
     curl -s "http://localhost:$EVERRUNS_PORT/internal/llm_query" \
       -d "{\"query\": \"$1\", \"context\": \"$2\"}"
   }
   ```

2. **Internal API endpoint** on the worker:
   - Worker exposes localhost-only endpoint for sandbox sub-calls
   - Routes to same `LlmQueryTool` implementation
   - Inherits session context, budget tracking

3. **This enables true symbolic recursion:**
   ```bash
   # LLM can now write this in virtual_bash:
   context=$(cat /session/files/document.txt)
   for i in $(seq 0 10000 ${#context}); do
     chunk="${context:$i:10000}"
     results+=("$(llm_query "Summarize this section" "$chunk")")
   done
   llm_query "Combine these summaries into a final answer" "${results[*]}"
   ```

### Phase 3: Recursive Sub-Agents

Full recursion where sub-calls get their own agent loop:

1. `llm_query` tool schedules a child workflow via `ScheduleChildWorkflow`
2. Child workflow runs Input → Reason → Act → Reason → Complete
3. Child has access to `llm_query` tool (depth > 0), enabling deeper recursion
4. Parent workflow waits for child completion
5. Tree of workflows tracked in durable execution engine

## Key Design Decisions

### 1. Sub-call model selection

Sub-calls should default to a cheaper/faster model to control costs. The agent's primary model handles orchestration while sub-calls handle focused semantic tasks.

**Recommended defaults:**
- Primary model: whatever the agent is configured with
- Sub-call model: configurable via `child_model`, default to `gpt-4.1-mini`
- Depth 2+: force cheapest available model

### 2. Context passing strategy

Two options:
- **Inline context**: Pass context as part of the `llm_query` tool argument (simple, works today)
- **Reference context**: Pass a session file path, sub-call loads from session storage (efficient for large contexts)

Start with inline, add reference support later for large contexts.

### 3. Sub-call statefulness

RLM paper uses stateful REPL where variables persist. In Everruns:
- Phase 1 (tool-based): Stateless sub-calls. Agent manages state via turn context.
- Phase 2 (bash-based): Stateful via bash environment variables.
- Phase 3 (sub-agent): Stateful via child session storage.

### 4. Depth limits and safety

| Control | Default | Purpose |
|---------|---------|---------|
| `max_depth` | 2 | Prevent infinite recursion |
| `max_calls_per_turn` | 50 | Prevent runaway costs |
| `budget_dollars` | 1.00 | Hard cost ceiling |
| `timeout_seconds` | 30 | Per-call timeout |
| `max_context_chars` | 500000 | Prevent oversized sub-calls |

### 5. Observability

Every sub-call generates:
- Event with parent_span_id linking to parent tool execution
- Usage metadata (tokens in, tokens out, model, latency)
- Cumulative cost tracking visible via `llm_cost` tool
- Turn-level aggregation in usage tracking

## Scope & Effort

| Phase | Scope | Dependencies |
|-------|-------|--------------|
| **Phase 1** | `LlmQueryCapability` with `llm_query`, `llm_batch_query`, `llm_cost` tools | LlmDriver access from tool context |
| **Phase 2** | `llm_query` bash function in virtual_bash sandbox | Internal API endpoint on worker |
| **Phase 3** | Recursive sub-agents via `ScheduleChildWorkflow` | Durable engine child workflow support |

Phase 1 is the foundation — it provides immediate RLM functionality while Phase 2 and 3 build on it incrementally.

## Open Questions

1. **Should sub-calls share the parent's message history?** YPI says no — each sub-call gets only the query + context provided. This is cleaner and cheaper.

2. **How to handle sub-call tool access?** Should sub-calls have access to other tools (bash, file system) or only respond with text? Start with text-only, expand later.

3. **Should sub-call results be stored as events?** Yes — enables observability and debugging. But keep events lightweight (don't store full context in event data).

4. **How to handle streaming?** Sub-call streaming is invisible to the user (it's inside a tool). Collect full response before returning. But emit progress events for long sub-calls.

## References

- [Recursive Language Models (arXiv:2512.24601)](https://arxiv.org/abs/2512.24601) — Zhang, Kraska & Khattab
- [alexzhang13/rlm](https://github.com/alexzhang13/rlm) — Official RLM implementation
- [alexzhang13/rlm-minimal](https://github.com/alexzhang13/rlm-minimal) — Minimal reference implementation
- [rawwerks/ypi](https://github.com/rawwerks/ypi) — Bash-based RLM coding agent
- [RLM blog post](https://alexzhang13.github.io/blog/2025/rlm/) — Alex Zhang's technical overview
- [Prime Intellect blog](https://www.primeintellect.ai/blog/rlm) — RLM as paradigm
