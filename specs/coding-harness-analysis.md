# Coding Harness Analysis

> State-of-art coding agent analysis and gap assessment for an Everruns Coding Harness.
> Theoretical exercise — no implementation decisions made.

## Agents Surveyed

| Agent | Vendor | Runtime | LLM Lock-in | OSS |
|---|---|---|---|---|
| Claude Code | Anthropic | Local CLI / IDE / Web | Claude only | Partial |
| Codex CLI | OpenAI | Local CLI + Cloud sandbox | OpenAI only | Yes |
| OpenCode (→ Crush) | Anomaly Innovations / Charm | Local CLI (Go TUI) | Multi-provider | Yes |
| Amp | Sourcegraph | IDE + CLI + Cloud | Claude Sonnet | No |
| Aider | Paul Gauthier | Local CLI | Multi-provider | Yes |
| Cline | Cline | IDE extension | Multi-provider | Yes |
| Gemini CLI | Google | Local CLI | Gemini only | Yes |
| Pi | Mario Zechner | Local CLI | Multi-provider | Yes |

---

## Universal Patterns

### Core Loop

Every agent follows ReAct: Think → Tool Call → Observe → Repeat. The model is the brain; tools are the interface. A harness must support this loop with durable state.

### Minimum Viable Tool Set

All agents converge on 4–6 core tools:

| Tool | Purpose | Who has it |
|---|---|---|
| **Read** | Read file contents | All |
| **Write/Edit** | Create or modify files | All |
| **Bash/Shell** | Execute commands | All |
| **Search (Grep/Glob)** | Find files and content | All |
| **Web Fetch** | HTTP requests | Most |
| **LSP** | Language-aware code intelligence | OpenCode only |

Pi demonstrates the minimum: exactly 4 tools (read, write, edit, bash) and <1K token system prompt.

### Extended Tool Set (Differentiators)

| Tool Category | Who | Everruns Status |
|---|---|---|
| Code graph / symbol index | Amp (Sourcegraph) | Missing |
| LSP integration | OpenCode | Missing |
| Browser automation | Cline, Claude Code (MCP) | Available via MCP |
| Image/screenshot understanding | Claude Code, Cursor | Partial (multimodal LLMs) |
| Web search | Claude Code, Codex, Gemini | Available via capability |

---

## Per-Agent Deep Dive

### Claude Code

**Architecture:** Agentic ReAct loop in terminal. 6 core tools (Read, Write, Edit, Bash, Glob, Grep). 200K context window.

**Key innovations:**
- **Subagents** — Up to 10 concurrent, isolated context windows, return summaries to parent. Primary context management strategy.
- **Hooks** — 21 lifecycle events (PreToolUse, PostToolUse, SessionStart, PermissionRequest, Stop). Shell commands that can block/modify/approve tool calls.
- **CLAUDE.md hierarchy** — Repo/project/user-level instruction files loaded at session start.
- **Compaction** — Manual (`/compact`) and automatic conversation summarization at ~60% capacity.
- **Agent SDK** — Python/TypeScript embedding. `-p` flag for headless JSON output.
- **Permission model** — Exit-code-based hook signaling (exit 2 = deny). Per-tool and per-subagent permissions.

**What Everruns can learn:** Hook system depth (21 events), per-command bash filtering, CLAUDE.md hierarchy pattern.

### Codex CLI

**Architecture:** Dual execution — cloud sandbox (isolated container per task) and local CLI. GPT-5.3-Codex model.

**Key innovations:**
- **Default-deny networking** — Network disabled by default in both local and cloud modes. Security-first posture.
- **Cloud sandbox** — Container provisioned with full repo, dependencies pre-installed (network allowed during setup, disabled during agent phase).
- **Four approval levels** — Suggest (review all), Auto Edit (auto-approve file edits), Full Auto (no approval), Granular (per-category rules for sandbox, exec-policy, MCP, skill-scripts).
- **Apply-patch editing** — Patch-based file modification with fuzzy matching.
- **MCP server mode** — Codex itself can run as an MCP server for orchestration by other agents.

**What Everruns can learn:** Default-deny security posture, dual local/cloud execution model, granular approval policies, sandbox networking control.

### OpenCode (→ Crush)

**Architecture:** Go CLI with Bubble Tea TUI. Client/server architecture. Provider-agnostic (75+ models). SQLite persistence.

**Key innovations:**
- **LSP integration** — `goToDefinition`, `findReferences`, `hover`, `documentSymbol`, `workspaceSymbol`, `goToImplementation`, call hierarchy. Real-time diagnostics fed back to LLM after edits.
- **Dual-agent architecture** — Plan agent (read-only, analysis) and Build agent (full tool access, changes). Architectural separation of intention from execution.
- **YAML-based agent definitions** — Declarative subagent configuration with per-agent tool permissions and system prompts.
- **Git-backed session review** — Visual diff, file status map, agent decision rationale.

**What Everruns can learn:** LSP integration is the standout — language-aware feedback loops catch errors before the next LLM call. Plan/Build separation as a harness pattern.

### Amp

**Architecture:** Built on Sourcegraph code graph and search. IDE-agnostic. Claude Sonnet 4. 168K context.

**Key innovations:**
- **Code graph** — Cross-repository semantic understanding via Sourcegraph's symbol index, reference tracking, dependency trees.
- **Persistent memory** — Cross-session knowledge of coding conventions, library usage, architectural decisions, testing patterns.
- **Team-first design** — Shared threads, context, and workflows. Cross-device sync via ampcode.com.
- **Context aggregation** — Dynamic updates from semantic data as code evolves. Graph querying for precise function/commit/class retrieval.

**What Everruns can learn:** Code graph as a capability (not just grep), persistent cross-session memory for coding patterns, team collaboration model.

### Other Notable Agents

**Aider:** Git-native (every change auto-committed). Multiple edit formats optimized per model. HTTP hooks for external services. Remote control from mobile/browser.

**Cline:** Auto-approve with per-category granularity. Browser automation via Computer Use. Plan/Act mode separation. 5M+ installs.

**Gemini CLI:** 1M token context window. Native OS sandboxing (macOS Seatbelt, Windows sandbox). A2A protocol support. Event-driven scheduler.

---

## Gap Analysis: Everruns vs. Coding Agents

### Where Everruns Leads

| Strength | Detail |
|---|---|
| **Durability** | PostgreSQL-backed durable execution. Agents survive disconnects, restarts, and can run overnight. No coding agent matches this. |
| **Multi-agent orchestration** | Named subagents, tool subsetting, shared filesystem, status tracking, messaging. More sophisticated than Claude Code's spawn-and-wait. |
| **Context management** | Observation masking (zero-cost), provider-native compaction, summarization fallback, cascading strategy, infinity context with `query_history`. Matches or exceeds all competitors. |
| **Composition model** | Harness → Agent → Session hierarchy with live inheritance, capability merging, and starter files. No competitor has this level of composability. |
| **Provider independence** | Multi-provider LLM drivers. Most top agents are vendor-locked. |
| **Programmatic API** | Full REST API, SSE streaming, CLI, SDK. Agents are API-first, not CLI-first. |

### Critical Gaps

#### P0 — Real Filesystem Access

**Problem:** Every coding agent operates on the real local filesystem — `git clone`, symlinks, `.gitignore`, file watchers, monorepos with 100K+ files, `node_modules`. Everruns VFS is PostgreSQL-backed BYTEA storage.

**Specific gaps:**
- No native symlink support
- No inotify/file watch events
- Performance at scale (monorepos with 50K+ files)
- No sparse checkout / partial clone awareness
- File sync adds latency vs. native FS
- Binary file handling (compiled outputs, images, .wasm)

**Mitigation paths:**
1. CLI file sync (exists) as default for "local coding" mode
2. Container with real FS mount for "cloud coding" mode (Codex model)
3. FUSE/bind-mount in worker container

#### P0 — Full Process Execution

**Problem:** Coding agents run `cargo build`, `npm install`, `docker compose up`, `pytest` — long-running processes with streaming output, port binding, signals, and host network interaction.

**Specific gaps:**
- Virtual bash sandboxing limits: daemon processes, port binding, PTY/interactive mode
- No background process management (dev servers alive across tool calls)
- No process group cleanup
- No resource limits (CPU/memory for builds)
- Streaming output during execution (not just final result)

**What competitors do:** Claude Code runs bash natively. Codex uses Docker/microVM. Amp runs in cloud VMs.

#### P0 — Test-Run-Fix Feedback Loop

**Problem:** The core coding agent workflow is: edit code → run tests → see failures → fix → re-run. This needs to be a first-class pattern, not ad-hoc bash commands.

**What competitors do:** Claude Code and Codex both have tight edit-test-fix loops as their primary workflow. OpenCode feeds LSP diagnostics back into the agent loop automatically.

**What Everruns needs:** A capability or pattern that orchestrates: edit → run configured test command → parse output → feed failures back to LLM → repeat.

### Significant Gaps

#### P1 — Git Workflow Capability

**Problem:** No built-in git capability. Git happens via bash or MCP.

**What a coding harness needs:**
- Branch management awareness (current branch, dirty state)
- Commit creation with conventional commit support
- PR lifecycle (create, respond to reviews, fix CI)
- Git diff as first-class concept (for context — "show me what changed")
- Auto-commit on changes (Aider model) as an option

**Mitigation:** Build as a capability on top of bash + GitHub MCP. Low architectural risk.

#### P1 — Per-Command Bash Permission Filtering

**Problem:** Claude Code's permission model filters at the bash command level — allow `npm test` but block `rm -rf /`. Codex has default-deny networking. Gemini CLI uses OS-native sandboxing.

**What Everruns has:** Tool-level policies (`auto` vs `requires_approval`), capability subsetting. But no command-level filtering within bash.

**What's needed:**
- Regex-based command allowlist/denylist
- Network isolation controls (default-deny option)
- Directory-scoped file permissions (allow edits in `src/` but not `.github/`)
- Command categorization (read-only vs. destructive vs. network)

#### P1 — Pre/Post Tool Hooks

**Problem:** Claude Code has 21 lifecycle events. Aider has lifecycle + HTTP hooks. Codex has approval policies. Everruns has capabilities and skills but no reactive hook system.

**What's needed:**
- `PreToolUse` / `PostToolUse` hooks that run shell commands or call webhooks
- Hooks that can block, modify, or approve tool calls
- Glob-activated rules (e.g., "when editing `*.sql`, run migration check")
- User-defined hooks in workspace (`.everruns/hooks/`)

#### P1 — Code Intelligence (LSP / Code Graph)

**Problem:** Top agents are moving beyond grep toward semantic code understanding.

| Approach | Agent | What it enables |
|---|---|---|
| LSP | OpenCode | goToDefinition, findReferences, type errors fed back to LLM |
| Code graph | Amp | Cross-repo symbol index, call hierarchy, dependency trees |
| Embeddings | Cursor/Windsurf | Semantic search over codebase |
| Multi-round grep | Claude Code | Parallel "Explore" agents doing iterative search |

**What Everruns needs:** At minimum, LSP as a capability that languages can plug into. Ideally, a code graph capability for cross-file understanding.

### Lower Priority Gaps

#### P2 — IDE Integration Protocol

IDEs (VS Code, JetBrains) are the primary coding surface. Cursor, Cline, Codex, and Amp all have IDE extensions. This is a separate product surface but important for adoption.

#### P2 — Codebase Indexing at Startup

Cursor and Windsurf pre-compute embeddings for fast retrieval. Amp uses Sourcegraph's code graph. This enables "find all implementations of this interface" without grep.

#### P2 — Interactive PTY Support

Running `vim`, `less`, interactive debuggers, `git rebase -i`. Complex streaming/terminal emulation problem.

#### P2 — OS-Native Sandboxing

Gemini CLI uses macOS Seatbelt and Windows sandbox for OS-level isolation. Codex uses container isolation. Important for security-sensitive deployments.

---

## Architectural Decision: Server-Side vs. Client-Side Execution

The fundamental question for a coding harness:

| Model | Example | Pros | Cons |
|---|---|---|---|
| **Client-side** | Claude Code, Aider, Pi | Native FS, native processes, zero infra cost, low latency | Dies on disconnect, no background execution, single-machine |
| **Server-side** | Codex cloud, Amp | Survives disconnects, scalable, isolated, auditable | High infra cost, FS sync overhead, network latency |
| **Hybrid** | Codex (dual mode) | Best of both | Complexity, two code paths |

**Everruns is uniquely positioned for hybrid:** Durable orchestration server-side (the engine) with native execution client-side (CLI + client-side tools). The client-side tools spec already supports pause-and-wait for client execution.

**Proposed model:**
- Server orchestrates the agent loop, maintains state, handles compaction
- Client-side tools handle: file read/write (native FS), bash (native process), git (native)
- Server-side tools handle: web fetch, MCP, code search, memory
- Background execution falls back to server-side VFS + virtual bash when client disconnects

---

## What Might We Miss?

### 1. Edit Format Diversity

Aider discovered that different LLMs perform best with different edit formats (whole-file, search-replace, unified diff, editor-diff). A coding harness may need to support multiple edit tool implementations optimized per model, not a single Edit tool.

### 2. Automatic Error Recovery Loops

OpenCode's LSP integration creates a tight feedback loop: edit → LSP diagnostics → auto-fix → repeat. Without this, errors only surface when the user runs tests or the LLM happens to check. A coding harness should support "post-edit validation hooks" that feed results back automatically.

### 3. Git as Source of Truth

Aider treats git as the undo system — every change auto-committed, easy rollback. Most agents treat git as a side effect. A coding harness could make git-native execution a mode: auto-checkpoint on each edit, easy rollback to any point.

### 4. Plan/Execute Separation

OpenCode and Cline architecturally separate planning (read-only, analysis) from execution (write, run). This isn't just UX — it's a security boundary. A coding harness could enforce this: Plan capability (read tools only) vs. Build capability (all tools).

### 5. Cost-Aware Model Routing

No current harness supports: "use cheap model for grep/exploration, expensive model for architecture decisions." Everruns' multi-provider drivers + per-message model override could enable this, but it needs a strategy layer.

### 6. Team Context Sharing

Amp's team-first model (shared threads, conventions, persistent memory) is unique. Most agents are single-developer tools. A coding harness for teams needs: shared coding conventions, team-wide memory, collaborative sessions.

### 7. MCP as Both Client and Server

Codex can run as an MCP server, allowing other agents to orchestrate it. Claude Code connects to MCP servers for external tools. A coding harness should support both directions — sessions that expose tools via MCP, and sessions that consume external MCP servers.

### 8. Default-Deny Security Posture

Codex's default-deny networking is a significant security choice. Most agents default-allow everything. A coding harness for enterprise needs default-deny with explicit allowlisting — Everruns' network access list capability is a start but needs to be the default, not opt-in.

### 9. Session Portability

Amp syncs threads across devices. Codex stores transcripts locally for resume. A coding harness needs session portability — start on laptop, continue on cloud, resume on phone. Everruns' server-side sessions already enable this, but the UX matters.

### 10. Workspace Detection & Auto-Configuration

No agent auto-detects "this is a Rust project, so I should run `cargo test`" at the harness level. AGENTS.md is manual. A coding harness could auto-detect: package.json → Node.js tools/conventions, Cargo.toml → Rust tools, pyproject.toml → Python tools. Pre-configure test commands, linters, formatters based on workspace analysis.

---

## Summary

Everruns has **strong structural advantages** (durability, multi-agent, context management, composition, provider independence) that no current coding agent matches. The gaps are primarily at the **infrastructure layer** (real FS, native processes, code intelligence) and **developer UX layer** (hooks, permissions, IDE integration).

The most promising path: a Coding Harness that inherits from Generic, adds git + test-runner + LSP capabilities, uses client-side tools for native FS/process execution, and falls back to server-side VFS/bash for background/cloud execution. This would combine Everruns' durability and orchestration strengths with the native execution model that makes Claude Code and Aider effective.
