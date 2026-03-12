# Subagent Architecture Analysis

Analysis of how top coding agents implement subagents/child agents, compared to everruns' current architecture, with recommendations.

## Current Everruns Architecture

### Agent Model: Flat, Capability-Composed

Everruns uses a **single-agent-per-session** model with no parent/child relationships:

```
Organization
  ├─ Harness (base config: prompt + capabilities)
  ├─ Agent (domain layer: prompt + capabilities + client tools)
  └─ Session (runtime: harness + optional agent + filesystem + tools)
       └─ RuntimeAgent (assembled: merged prompt + merged tools + model)
```

**Key properties:**
- Agents are **persisted** (PostgreSQL), not dynamic
- Sessions are isolated (own filesystem, KV store, secrets, SQL DB)
- No session-to-session communication
- No agent spawning during execution
- Capabilities are **registry-based** (Rust code, not DB-driven)
- Tools come exclusively from capabilities (not directly assigned)

### What We Have That's Relevant

| Feature | Status | Notes |
|---------|--------|-------|
| `platform_management` capability | Exists | Can create sessions/agents programmatically via tools |
| `session_interact` tool | Exists | Send messages to other sessions, wait for idle |
| Session filesystem | Exists | Per-session isolated VFS backed by PostgreSQL |
| MCP integration | Exists | External tool servers as virtual capabilities |
| Skills system | Exists | VFS-discovered + registry-based skill loading |
| Durable execution engine | Exists | PostgreSQL-backed workflow orchestration |

### Gap: No In-Turn Agent Delegation

The `platform_management` capability can create sessions and interact with them, but:
- It's a **user-facing** tool (Platform Chat harness), not an execution primitive
- No orchestrator pattern (no "spawn subagent, collect result, continue")
- No shared context between parent and child
- No automatic cleanup of child sessions
- No parallel subagent execution within a single turn

---

## Top Coding Agent Architectures

### Claude Code (Anthropic)

**Model:** Orchestrator with built-in specialized subagents + custom subagent definitions.

**Built-in subagents:**
- **Explore** — read-only codebase search (fast model, restricted tools: Glob, Grep, Read, Bash read-only)
- **Plan** — research agent for plan mode (read-only tools, returns structured plan)
- **Worker fork** — executes directive without spawning further subagents
- **Verification specialist** — adversarial testing (builds, tests, linters → PASS/FAIL/PARTIAL)

**Key design decisions:**
- Each subagent has **own context window** (prevents context pollution)
- Subagents inherit parent permissions but get **restricted tool sets**
- Parent sends task description, subagent returns single result message
- **No nesting** (worker fork explicitly blocks further spawning)
- Custom subagents defined in `.claude/agents/` with YAML config (tools, model, memory dir)
- **Agent Teams** (Feb 2026): multiple agents that communicate, not just delegate

**What matters for us:**
- Context isolation is the primary motivation
- Tool restriction per subagent (not all tools available)
- Read-only vs read-write distinction
- Single result message back to parent (clean interface)

### Cursor

**Model:** Orchestrator-worker pattern with parallel subagents + background agents.

**Subagents (v2.4, Jan 2026):**
- Spawned in parallel (Subagent A: docs, B: code, C: terminal)
- Each has own context window and state
- ~2x speedup on migration tasks (9 min vs 17 min parallel vs serial)

**Background Agents:**
- Fork-based workflow (creates branch, works independently, proposes PR)
- Cloud sandbox environment preloaded with repo
- Admin controls (allow list for who can start agents)

**Automations (Mar 2026):**
- Event-triggered agents (code push, Slack message, PagerDuty, timer)
- Hundreds of automations per hour

**What matters for us:**
- Parallel subagent execution is the killer feature
- Background agents = long-running async work
- Event triggers = scheduled tasks (we already have this via `session_schedule`)

### OpenAI Codex

**Model:** Cloud sandbox per task, multi-agent via spawning + result collection.

**Architecture:**
- Each task runs in own cloud sandbox (preloaded repo)
- Sub-agents spawned for specific tasks, results collected into one response
- Different models for different agents (strong reasoning vs fast exploration)
- AGENTS.md + MCP for repo-specific configuration
- CLI can run as MCP server (enables external orchestration)

**Context management motivation:**
- Context pollution: useful info buried under noise
- Context rot: performance degrades as conversation fills
- Solution: move noisy work off main thread

**What matters for us:**
- Model selection per subagent (we support model override per session)
- Context management is the universal motivation
- MCP-as-orchestration-layer pattern

---

## Analysis: What We Can Do

### Dynamic vs Persisted Agents

**Current state:** All agents are persisted (created via API, stored in PostgreSQL).

**What competitors do:** Subagents are **dynamic** — spawned during execution, not pre-created. They exist only for the duration of a task.

**Recommendation:** Support both:
1. **Persisted agents** (current) — for reusable domain-specific configurations
2. **Ephemeral subagents** — spawned during turn execution, auto-cleaned after parent turn completes

Implementation sketch:
- New `SubagentCapability` with a `spawn_subagent` tool
- Tool params: `task`, `tools` (subset of parent's tools), `model` (optional), `mode` (read_only | read_write)
- Creates ephemeral session with subset of parent's capabilities
- Runs to completion, returns single result to parent turn
- Session + filesystem cleaned up after result collected (or after TTL)

### Access to Session File Storage

**Current state:** Each session has isolated VFS. No sharing.

**What competitors do:**
- Claude Code: subagents share the same filesystem (same working directory)
- Codex: each sandbox preloaded with repo snapshot
- Cursor: subagents share workspace

**Recommendation:** Two modes:
1. **Shared filesystem** (default for subagents) — subagent mounts parent's `/workspace` read-only or read-write
2. **Snapshot filesystem** — subagent gets COW copy of parent's workspace (for isolation)

Implementation:
- `session_files` already supports `is_readonly`
- Add `parent_session_id` FK to sessions table (nullable)
- Subagent filesystem queries fall through to parent when file not found locally
- Write-back: on subagent completion, changed files optionally merged to parent

### Access to Tools of the Main Agent

**Current state:** Tools come from capabilities. No tool subsetting within a session.

**What competitors do:**
- Claude Code: explicit tool restriction per subagent (Explore gets Glob+Grep+Read only)
- Codex: different tool sets per spawned agent
- All: read-only subagents are common pattern

**Recommendation:** Tool subsetting for subagents:
- `spawn_subagent` tool accepts `allowed_tools: Vec<String>` (allowlist)
- Or `allowed_capabilities: Vec<String>` (capability-level restriction)
- Default: inherit all parent capabilities
- Common presets: `read_only` (file read + grep + bash read-only), `full` (all parent tools)

### Parallel Subagent Execution

**What competitors do:** All top agents support parallel subagent execution.

**Recommendation:** The `ActAtom` already executes tool calls in parallel via `futures::join_all`. If multiple `spawn_subagent` tool calls appear in same LLM response, they'd naturally run in parallel. No architectural change needed — just ensure subagent execution is async-compatible.

### Proposed Architecture

```
Turn Execution (parent session)
  │
  ├─ ReasonAtom → LLM returns tool_calls including spawn_subagent(...)
  │
  └─ ActAtom (parallel execution)
       ├─ spawn_subagent("explore codebase for auth patterns",
       │    tools: ["read_file", "grep_files", "list_directory"],
       │    model: "fast")
       │    → Creates ephemeral session
       │    → Mounts parent /workspace read-only
       │    → Runs own turn loop (Input → Reason → Act → ... → Complete)
       │    → Returns single text result
       │
       ├─ spawn_subagent("write unit tests for auth module",
       │    tools: ["read_file", "write_file", "bash"],
       │    model: "strong")
       │    → Creates ephemeral session
       │    → Mounts parent /workspace read-write
       │    → Runs own turn loop
       │    → Returns result + file changes merged back
       │
       └─ regular_tool("get_current_time")
            → Normal tool execution
```

### Priority Ranking

| Feature | Impact | Effort | Priority |
|---------|--------|--------|----------|
| `spawn_subagent` tool (basic) | High | Medium | P0 |
| Shared/inherited filesystem | High | Medium | P0 |
| Tool subsetting (allowlist) | Medium | Low | P1 |
| Read-only mode for subagents | Medium | Low | P1 |
| Model selection per subagent | Low | Low | P1 |
| Ephemeral session cleanup | Medium | Low | P1 |
| Parallel execution (natural) | High | Zero | Free |
| Snapshot/COW filesystem | Low | High | P2 |
| Background agents (long-running) | Medium | High | P2 |
| Event-triggered agents | Medium | Medium | P2 (have `session_schedule`) |

### Implementation Path

**Phase 1: Core Subagent Primitive**
- New `subagent` capability with `spawn_subagent` tool
- Ephemeral session creation (parent_session_id FK)
- Filesystem inheritance (shared mount of parent workspace)
- Tool/capability subsetting via allowlist
- Max iterations guard (prevent runaway subagents)
- Auto-cleanup on completion

**Phase 2: Presets & Ergonomics**
- Built-in subagent presets: `explore` (read-only, fast model), `worker` (read-write, no nesting)
- Subagent nesting depth limit (max 1-2 levels)
- Result summarization (truncate long subagent outputs)
- Subagent events (SSE: subagent.started, subagent.completed)

**Phase 3: Background & Orchestration**
- Long-running background subagents (via durable engine)
- Cross-session communication primitives
- Event-triggered subagent spawning (extend `session_schedule`)

---

## Sources

- [Claude Code Subagents Docs](https://code.claude.com/docs/en/sub-agents)
- [Claude Code System Prompts (GitHub)](https://github.com/Piebald-AI/claude-code-system-prompts)
- [Claude Code Agent Teams Guide](https://www.heyuan110.com/posts/ai/2026-02-28-claude-code-teams-guide/)
- [Cursor Background Agents](https://docs.cursor.com/en/background-agent)
- [Cursor 2.4 Subagents](https://www.aimakers.co/blog/cursor-2-4-subagents/)
- [Cursor Automations (TechCrunch)](https://techcrunch.com/2026/03/05/cursor-is-rolling-out-a-new-system-for-agentic-coding/)
- [OpenAI Codex Multi-Agents](https://developers.openai.com/codex/concepts/multi-agents/)
- [VS Code Multi-Agent Development](https://code.visualstudio.com/blogs/2026/02/05/multi-agent-development)
