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
| `session_schedule` capability | Exists | Cron-based scheduled tasks |

### Gap: No In-Turn Agent Delegation

The `platform_management` capability can create sessions and interact with them, but:
- It's a **user-facing** tool (Platform Chat harness), not an execution primitive
- No orchestrator pattern (no "spawn subagent, collect result, continue")
- No shared context between parent and child
- No automatic cleanup of child sessions
- No parallel subagent execution within a single turn
- No subagent status tracking or lifecycle management

---

## Top Coding Agent Architectures

### Claude Code (Anthropic)

**Model:** Agent tool as first-class primitive. Parent agent delegates via `Agent` tool calls. Each subagent runs in own context window.

**Architecture details (from [official docs](https://code.claude.com/docs/en/sub-agents)):**

The `Agent` tool is a regular tool in the agent loop. When the LLM emits an `Agent` tool call, Claude Code:
1. Creates a new agent instance with its own context window
2. Injects the subagent's system prompt (from markdown file frontmatter body)
3. Restricts available tools per subagent config
4. Runs the subagent's own agent loop (Reason → Act → ... → Complete)
5. Returns a single result message back to the parent

**Built-in subagents:**

| Subagent | Model | Tools | Purpose |
|----------|-------|-------|---------|
| **Explore** | Haiku (fast) | Read-only (denied Write, Edit) | Codebase search, file discovery |
| **Plan** | Inherits | Read-only (denied Write, Edit) | Research for plan mode |
| **General-purpose** | Inherits | All tools | Complex multi-step tasks |
| **Bash** | Inherits | Terminal commands | Separate context for shell |
| **Claude Code Guide** | Haiku | Read-only | Questions about Claude Code |

**Custom subagent configuration (YAML frontmatter in `.claude/agents/*.md`):**

```yaml
name: code-reviewer          # unique identifier
description: Reviews code... # when to delegate (LLM reads this)
tools: Read, Grep, Glob, Bash  # allowlist (inherits all if omitted)
disallowedTools: Write, Edit    # denylist (removed from inherited)
model: sonnet                   # sonnet|opus|haiku|inherit|full-model-id
permissionMode: default         # default|acceptEdits|dontAsk|bypassPermissions|plan
maxTurns: 20                    # max agentic turns
memory: user                    # persistent memory scope: user|project|local
background: true                # always run as background task
isolation: worktree             # git worktree isolation
mcpServers:                     # scoped MCP servers
  - playwright:
      type: stdio
      command: npx
      args: ["-y", "@playwright/mcp@latest"]
  - github                      # reference existing server by name
skills:                         # preloaded skill content
  - api-conventions
hooks:                          # lifecycle hooks scoped to subagent
  PreToolUse:
    - matcher: "Bash"
      hooks:
        - type: command
          command: "./scripts/validate.sh"
```

**Key design decisions:**
- **No nesting**: subagents cannot spawn other subagents (prevents runaway)
- **Tool restriction is first-class**: `tools` allowlist + `disallowedTools` denylist
- **Foreground vs background**: foreground blocks parent; background runs concurrently
- **Background permission pre-approval**: before launching background subagent, all needed permissions collected upfront; auto-denies anything not pre-approved
- **Resumable**: each subagent gets an `agentId`; parent can resume it later with full context preserved
- **Persistent memory**: subagent can build knowledge across sessions via memory directory
- **Git worktree isolation**: `isolation: worktree` gives subagent isolated repo copy
- **MCP scoping**: MCP servers can be scoped to specific subagents (not visible to parent)
- **Hooks**: subagent-scoped `PreToolUse`/`PostToolUse`/`Stop` hooks + parent-level `SubagentStart`/`SubagentStop`
- **Auto-compaction**: subagents auto-compact at ~95% context capacity
- **Transcript persistence**: stored as `agent-{agentId}.jsonl`, survives main conversation compaction

**Permission flow:**
- Subagents inherit parent conversation permissions
- Can override with `permissionMode` in config
- Parent `bypassPermissions` takes precedence (cannot be restricted)
- `Agent(agent-name)` permission rules control which subagents can be spawned
- `tools` field on main agent config can restrict: `tools: Agent(worker, researcher), Read, Bash`

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

### OpenAI Codex

**Model:** Cloud sandbox per task, multi-agent via spawning + result collection.

**Architecture:**
- Each task runs in own cloud sandbox (preloaded repo)
- Sub-agents spawned for specific tasks, results collected into one response
- Different models per agent (gpt-5.3-codex for reasoning, gpt-5.3-codex-spark for speed)
- AGENTS.md + MCP for repo-specific configuration
- CLI runs as MCP server (enables external orchestration)

**Context management motivation:**
- Context pollution: useful info buried under noise
- Context rot: performance degrades as conversation fills
- Solution: move noisy work off main thread

---

## Analysis: What We Can Do

### Dynamic vs Persisted Agents

**Current state:** All agents are persisted (created via API, stored in PostgreSQL).

**What competitors do:** Subagents are **dynamic** — spawned during execution from inline config or file-based definitions. They exist for the duration of a task (foreground) or until completion (background).

**Recommendation:** Support both:
1. **Persisted agents** (current) — reusable domain-specific configurations
2. **Ephemeral subagents** — spawned during turn execution with inline config
3. **Template-based subagents** — defined in workspace files (like `.claude/agents/*.md`), discovered via VFS

Implementation:
- New `SubagentCapability` with `spawn_subagent` tool
- Subagent config can reference a persisted agent_id OR provide inline config (prompt, tools, model)
- Template discovery via `agent_instructions`-style VFS scanning (`/.agents/subagents/`)
- Ephemeral sessions auto-cleaned after TTL or parent completion

### Access to Session File Storage

**Current state:** Each session has isolated VFS. No sharing.

**What competitors do:**
- Claude Code: subagents share the same working directory (filesystem)
- Claude Code `isolation: worktree`: git worktree gives isolated copy
- Codex: each sandbox preloaded with repo snapshot
- Cursor: subagents share workspace

**Recommendation:** Two modes:
1. **Shared filesystem** (default) — subagent mounts parent's `/workspace` read-only or read-write
2. **Isolated filesystem** — subagent gets snapshot of parent's workspace (for worktree-like isolation)

Implementation:
- `session_files` already supports `is_readonly`
- Add `parent_session_id` FK to sessions table (nullable)
- Shared mode: subagent filesystem queries fall through to parent when file not found locally
- Write-back: on subagent completion, changed files optionally merged to parent
- Isolated mode: COW snapshot of parent workspace at subagent creation time

### Access to Tools of the Main Agent

**Current state:** Tools come from capabilities. No tool subsetting within a session.

**What competitors do:**
- Claude Code: `tools` allowlist + `disallowedTools` denylist per subagent
- Claude Code: presets like read-only (no Write/Edit) are common
- Codex: different tool sets per spawned agent
- All: read-only subagents are the most common pattern

**Recommendation:** Tool subsetting matching Claude Code's model:
- `spawn_subagent` accepts `allowed_tools: Vec<String>` (allowlist)
- `spawn_subagent` accepts `disallowed_tools: Vec<String>` (denylist, removed from inherited)
- Default: inherit all parent capabilities
- Common presets built-in: `read_only`, `full`
- Capability-level restriction also supported: `allowed_capabilities: Vec<String>`

### Background/Async Subagents

**What competitors do:**
- Claude Code: `background: true` flag; permissions pre-approved upfront; auto-denies unpermitted tools; parent notified on completion
- Claude Code: resumable via `agentId`
- Cursor: background agents fork branch, work independently, propose PR
- Cursor automations: event-triggered, fully autonomous

**Recommendation:** First-class background subagents with status tracking:

**Host ↔ Subagent Communication Model:**

```
Parent Session
  │
  ├─ list_subagents() → [{id, status, task, created_at, model}]
  │
  ├─ spawn_subagent(task, config)
  │    → Creates child session
  │    → Returns subagent_id immediately (if background)
  │    → Or blocks until completion (if foreground)
  │
  ├─ get_subagent_status(subagent_id)
  │    → {status: running|completed|failed, progress, result}
  │
  ├─ cancel_subagent(subagent_id)
  │    → Cancels running subagent
  │
  └─ resume_subagent(subagent_id, additional_context)
       → Resumes completed/failed subagent with new instructions
```

**Subagent Status Lifecycle:**

```
Spawning → Running → Completed
                  → Failed
                  → Cancelled
                  → MaxIterationsReached
```

**SSE Events:**
- `subagent.spawned` — new subagent created (includes subagent_id, task, mode)
- `subagent.progress` — intermediate updates (iteration count, current tool)
- `subagent.completed` — subagent finished (includes result summary)
- `subagent.failed` — subagent errored
- `subagent.cancelled` — subagent was cancelled

**Implementation:**
- Background subagents run via durable execution engine (PostgreSQL-backed, survives restarts)
- Parent session stores `child_subagents: Vec<SubagentRef>` (id, status, task summary)
- Subagent results stored in `subagent_results` table (session_id, subagent_id, result, status)
- Parent LLM sees subagent status in system prompt or via `list_subagents` tool
- On subagent completion, parent session receives notification (triggers new turn if idle)

### Parallel Subagent Execution

**What competitors do:** All top agents support parallel subagent execution.

**Recommendation:** `ActAtom` already executes tool calls in parallel via `futures::join_all`. If multiple `spawn_subagent` tool calls appear in same LLM response, they naturally run in parallel. No architectural change needed for foreground subagents. Background subagents are inherently parallel (fire-and-forget).

### Subagent Nesting

**What competitors do:** Claude Code explicitly blocks nesting (subagents cannot spawn subagents).

**Recommendation:** Block nesting initially. Simple depth check:
- `spawn_subagent` tool checks if current session has `parent_session_id` set
- If yes, reject with error "Subagents cannot spawn other subagents"
- Future: allow 1 level of nesting with explicit opt-in

---

## Proposed Architecture

### Data Model Changes

```sql
-- New columns on sessions table
ALTER TABLE sessions ADD COLUMN parent_session_id UUID REFERENCES sessions(id);
ALTER TABLE sessions ADD COLUMN subagent_config JSONB;  -- inline config for ephemeral subagents
ALTER TABLE sessions ADD COLUMN subagent_status TEXT;    -- spawning|running|completed|failed|cancelled

-- Subagent results (separate table for clean querying)
CREATE TABLE subagent_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    parent_session_id UUID NOT NULL REFERENCES sessions(id),
    subagent_session_id UUID NOT NULL REFERENCES sessions(id),
    task TEXT NOT NULL,
    status TEXT NOT NULL,  -- completed|failed|cancelled|max_iterations
    result TEXT,           -- final result text
    iterations INTEGER,
    tool_calls_count INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);
```

### Capability Design

```rust
// New SubagentCapability
pub struct SubagentCapability;

impl Capability for SubagentCapability {
    fn id(&self) -> &str { "subagents" }
    fn name(&self) -> &str { "Subagents" }
    fn description(&self) -> &str {
        "Spawn and manage subagents for parallel/background task execution"
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SpawnSubagentTool),
            Box::new(ListSubagentsTool),
            Box::new(GetSubagentStatusTool),
            Box::new(CancelSubagentTool),
            Box::new(ResumeSubagentTool),
        ]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["subagents"]
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SUBAGENT_SYSTEM_PROMPT)
    }
}
```

### Tool Schemas

```json
{
  "name": "spawn_subagent",
  "description": "Spawn a subagent to handle a specific task. Runs in own context window.",
  "parameters": {
    "task": "string (required) — task description for the subagent",
    "mode": "enum: foreground|background (default: foreground)",
    "config": {
      "system_prompt": "string (optional) — custom system prompt",
      "agent_id": "string (optional) — reference persisted agent",
      "template": "string (optional) — reference workspace template name",
      "allowed_tools": ["string"] ,
      "disallowed_tools": ["string"],
      "allowed_capabilities": ["string"],
      "model": "string (optional) — model override",
      "max_turns": "integer (optional, default: 20)",
      "filesystem_mode": "enum: shared_rw|shared_ro|isolated (default: shared_rw)"
    }
  }
}

{
  "name": "list_subagents",
  "description": "List all subagents spawned by this session",
  "parameters": {
    "status_filter": "enum: all|running|completed|failed (optional)"
  }
}

{
  "name": "get_subagent_status",
  "description": "Get detailed status and result of a specific subagent",
  "parameters": {
    "subagent_id": "string (required)"
  }
}

{
  "name": "cancel_subagent",
  "description": "Cancel a running background subagent",
  "parameters": {
    "subagent_id": "string (required)"
  }
}

{
  "name": "resume_subagent",
  "description": "Resume a completed/failed subagent with additional context",
  "parameters": {
    "subagent_id": "string (required)",
    "additional_context": "string (required)"
  }
}
```

### Execution Flow

```
Parent Session Turn
  │
  ├─ ReasonAtom → LLM returns tool_calls
  │
  └─ ActAtom (parallel execution of all tool calls)
       │
       ├─ spawn_subagent(task: "explore auth", mode: "foreground",
       │    config: { allowed_tools: ["read_file", "grep_files"], model: "fast" })
       │    │
       │    ├─ Create ephemeral child session (parent_session_id = parent.id)
       │    ├─ Mount parent /workspace (shared_ro)
       │    ├─ Build RuntimeAgent with restricted tools + custom/default prompt
       │    ├─ Run child turn loop: Input → Reason → Act → ... → Complete
       │    ├─ Store result in subagent_results
       │    └─ Return result text to parent ActAtom
       │
       ├─ spawn_subagent(task: "run tests", mode: "background",
       │    config: { allowed_tools: ["bash", "read_file"], model: "inherit" })
       │    │
       │    ├─ Create child session (parent_session_id = parent.id)
       │    ├─ Mount parent /workspace (shared_rw)
       │    ├─ Enqueue in durable execution engine
       │    ├─ Return immediately: { subagent_id: "sub_xxx", status: "running" }
       │    └─ (runs asynchronously, emits SSE events)
       │
       └─ regular_tool("get_current_time")
            → Normal tool execution

Later turn (parent checks on background subagent):
  │
  ├─ ReasonAtom → LLM calls list_subagents() or get_subagent_status()
  └─ ActAtom → Returns current status/results
```

### Priority Ranking

| Feature | Impact | Effort | Priority |
|---------|--------|--------|----------|
| `spawn_subagent` tool (foreground) | High | Medium | P0 |
| Shared/inherited filesystem | High | Medium | P0 |
| Tool/capability subsetting | High | Low | P0 |
| Background subagent mode | High | Medium | P0 |
| `list_subagents` / `get_subagent_status` tools | High | Low | P0 |
| Subagent SSE events | Medium | Low | P0 |
| Nesting prevention | Medium | Trivial | P0 |
| Model selection per subagent | Medium | Low | P1 |
| `cancel_subagent` / `resume_subagent` | Medium | Medium | P1 |
| Ephemeral session cleanup (TTL) | Medium | Low | P1 |
| Read-only filesystem mode | Medium | Low | P1 |
| Workspace template discovery (VFS) | Low | Medium | P1 |
| Parallel execution (natural via ActAtom) | High | Zero | Free |
| Isolated/COW filesystem | Low | High | P2 |
| Event-triggered subagent spawning | Medium | Medium | P2 |
| Persistent subagent memory | Low | Medium | P2 |

### Implementation Path

**Phase 1: Core Subagent Primitive + Background Mode**
- `SubagentCapability` with all 5 tools
- Ephemeral session creation (`parent_session_id` FK, `subagent_config` JSONB)
- Subagent status lifecycle (`subagent_status` column)
- Filesystem inheritance (shared mount of parent workspace, rw/ro modes)
- Tool/capability subsetting via allowlist + denylist
- Max turns guard (prevent runaway)
- Foreground: blocks parent tool call until complete
- Background: enqueue via durable engine, return immediately
- `list_subagents` + `get_subagent_status` for host ↔ subagent communication
- SSE events: `subagent.spawned`, `subagent.completed`, `subagent.failed`
- Nesting depth check (block subagents from spawning subagents)

**Phase 2: Lifecycle Management**
- `cancel_subagent` with graceful shutdown
- `resume_subagent` with context preservation
- Auto-cleanup: TTL-based ephemeral session garbage collection
- Built-in subagent presets: `explore` (read-only, fast model), `worker` (read-write, no nesting)
- `subagent.progress` SSE events (iteration count, current tool)
- Workspace template discovery from VFS (`/.agents/subagents/*.md`)

**Phase 3: Advanced Orchestration**
- Persistent subagent memory (cross-session knowledge, like Claude Code's memory dir)
- Notification to idle parent when background subagent completes (trigger new turn)
- Event-triggered subagent spawning (extend `session_schedule`)
- Subagent-scoped MCP servers (connect on start, disconnect on finish)
- Isolated/COW filesystem mode

---

## Key Differences From Claude Code

| Aspect | Claude Code | Everruns (proposed) |
|--------|------------|-------------------|
| Runtime | Local CLI process | Server-side, durable engine backed |
| Subagent storage | Local filesystem (jsonl transcripts) | PostgreSQL (sessions + subagent_results) |
| Background mode | Local async task | Durable workflow (survives server restarts) |
| Filesystem | Shared OS filesystem | VFS with explicit sharing modes |
| MCP scoping | Per-subagent inline definitions | Per-subagent capability config |
| Hooks | Shell command hooks | Capability-level hooks (future) |
| Memory | File-based (MEMORY.md) | Session KV store or VFS |
| Nesting | Blocked | Blocked (same) |
| Permissions | Inherited + overridable per subagent | Capability subsetting |

The key advantage of server-side implementation: background subagents survive disconnects, are observable via API, and integrate with the durable execution engine for reliability.

---

## Sources

- [Claude Code Subagents Docs](https://code.claude.com/docs/en/sub-agents)
- [Claude Code Permissions — Agent Subagents](https://code.claude.com/docs/en/permissions#agent-subagents)
- [Claude Code System Prompts (GitHub)](https://github.com/Piebald-AI/claude-code-system-prompts)
- [Claude Code Agent Teams Guide](https://www.heyuan110.com/posts/ai/2026-02-28-claude-code-teams-guide/)
- [Cursor Background Agents](https://docs.cursor.com/en/background-agent)
- [Cursor 2.4 Subagents](https://www.aimakers.co/blog/cursor-2-4-subagents/)
- [Cursor Automations (TechCrunch)](https://techcrunch.com/2026/03/05/cursor-is-rolling-out-a-new-system-for-agentic-coding/)
- [OpenAI Codex Multi-Agents](https://developers.openai.com/codex/concepts/multi-agents/)
- [VS Code Multi-Agent Development](https://code.visualstudio.com/blogs/2026/02/05/multi-agent-development)
