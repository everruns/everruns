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

#### P0 — Real Filesystem & Process Execution (Two-Level Architecture)

**Reality:** Everruns operates at two levels:
1. **Virtual bash + VFS** — Lightweight, PostgreSQL-backed, always available. For configuration, notes, artifacts, simple scripts.
2. **Sandbox (Daytona, E2B, etc.)** — Real filesystem, real processes. For actual coding work.

This two-level model is the architecture. A Coding Harness defaults to sandbox mode for real FS/process access. The gap is not "missing real FS" — it's ensuring the sandbox experience is as smooth as native CLI agents (Claude Code, Aider).

**Sandbox parity gaps vs. native CLI agents:**
- Streaming output during long builds (not just final result)
- Background process management (dev servers alive across tool calls)
- Port forwarding / URL preview for web development
- Process group cleanup on session end
- Resource limits visibility (CPU/memory for builds)

#### P0 — Test-Run-Fix Feedback Loop

**Problem:** The core coding agent workflow is: edit code → run tests → see failures → fix → re-run. This needs to be a first-class pattern, not ad-hoc bash commands.

**What competitors do:** Claude Code and Codex both have tight edit-test-fix loops as their primary workflow. OpenCode feeds LSP diagnostics back into the agent loop automatically.

**What Everruns needs:** A capability or pattern that orchestrates: edit → run configured test command → parse output → feed failures back to LLM → repeat.

### Significant Gaps

#### P1 — Git Workflow Capability

**Approach:** Git operates in sandbox via native bash — the Claude Code way. No need for a dedicated git capability; the sandbox has real git.

**What the system prompt and AGENTS.md should cover:**
- Git safety rules (never force push, never skip hooks, conventional commits)
- Branch management awareness injected as environment context
- PR lifecycle via GitHub MCP or `gh` CLI in sandbox

**Optional enhancements:**
- Git diff as context injection (auto-inject `git diff` into LLM context after edits)
- Auto-commit on changes (Aider model) as a configurable mode
- PR review response workflow as a skill

#### P1 — Per-Command Bash Permission Filtering (TABLED)

**Status:** Tabled. Currently operating in YOLO mode (full auto-approve). Will revisit when enterprise/multi-tenant use cases demand it.

**For reference, what competitors do:**
- Claude Code: regex-based command filtering, exit-code-based hook signaling
- Codex: default-deny networking, four approval tiers
- Gemini CLI: OS-native sandboxing (macOS Seatbelt)

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

**What Everruns needs:** Explore the Claude Code approach first — parallel subagents doing multi-round grep/glob in sandbox. This is the cheapest path and leverages existing subagent infrastructure. LSP and code graph are P2 enhancements if subagent-based exploration proves insufficient.

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

## System Prompts

The current Generic harness prompt is minimal: `"You are a helpful assistant."` + instruction hierarchy + capability-injected XML sections. A Coding Harness needs significantly more.

### What Coding Agents Put in Their System Prompts

| Agent | Base Size | Total w/ Tools | Structure |
|---|---|---|---|
| Claude Code | ~3K tokens | ~14-17K tokens | Flat sections, tool schemas dominate |
| Codex CLI | ~2-3K tokens | ~4-6K tokens | Markdown, open-source in `prompt.md` |
| Cursor | ~1.25K tokens | ~5-8K tokens | XML-like tags (`<communication>`, `<tool_calling>`, `<making_code_changes>`) |
| Aider | ~1-2K tokens | ~1-2K tokens | No tool-calling; structured text output (SEARCH/REPLACE) |
| Devin | Large | Large | Three modes: planning, standard, edit |

### Universal System Prompt Sections

Every coding agent's system prompt contains these sections:

**1. Identity/Role** — Specific persona and capability boundaries. Claude Code: "interactive CLI tool for software engineering tasks". Cursor: "powerful agentic AI coding assistant". Devin: "autonomous AI software engineer".

**2. Tool Selection Steering** (~550 tokens in Claude Code) — The highest-impact section. Steers the LLM to use the right tool:
- "Use Read instead of cat, Edit instead of sed, Grep instead of grep"
- "Prefer Edit over Write for existing files"
- "Read files before editing them"
- "Make parallel tool calls for independent operations"

**3. Code Quality Guardrails** — Prevents over-engineering:
- Don't add features beyond what was asked
- Don't add unnecessary error handling, comments, or type annotations
- Don't create helpers/abstractions for one-time operations
- Fix root cause, not symptoms
- OWASP top 10 awareness

**4. Reversibility & Safety** (~540 tokens in Claude Code) — "Consider reversibility and blast radius of actions":
- Categorize actions as safe (read, edit) vs. risky (delete, force push)
- Never destructive without confirmation
- Create new commits rather than amending

**5. Git Safety Protocol** — Every agent has explicit git rules:
- Never force push
- Never skip hooks (--no-verify)
- Never amend published commits
- Specific commit message format (conventional commits, HEREDOC)
- Claude Code: never commit without explicit user request

**6. Output Style** — Anti-verbosity tuning:
- Concise, no filler, lead with answer
- Reference code as `file_path:line_number`
- Never mention tool names to users
- Markdown formatting

**7. Error Recovery** — Retry discipline:
- Read the error before retrying
- Don't retry identical failing commands
- Cap retry loops (Cursor: 3 max)
- Address root causes, not symptoms

**8. Context Injection Points** — Where project-specific instructions load:

| Agent | Mechanism | Injection Point |
|---|---|---|
| Claude Code | `CLAUDE.md` hierarchy | `system-reminder` messages, survives compaction |
| Codex CLI | `AGENTS.md` files | User-message role, directory-scoped precedence |
| Cursor | Custom rules | Separate user-role message (prevents prompt injection) |
| Aider | `.aider.conf.yml` + repo map | Per-session, tree-sitter AST of codebase |
| Copilot | `.github/instructions/*.instructions.md` | Glob-scoped per file pattern |

### Implications for Everruns Coding Harness

The Generic harness contributes capability prompts via XML tags (`<capability id="...">`) — good structure. But a Coding Harness needs a **domain-specific base prompt** covering:

1. **Tool selection steering** — Which tools to use for sandbox vs. workspace operations. "Use sandbox file tools for code, workspace VFS for configuration." This is the single highest-impact addition.
2. **Code quality guardrails** — Prevents the LLM from over-engineering, adding unnecessary comments, or making changes beyond scope.
3. **Git safety protocol** — Commit rules, branch safety, force-push prevention.
4. **Error recovery discipline** — Read errors, don't blind-retry, cap loops.
5. **Output format** — `file:line` references, concise style, no tool name leaks.

### Key Insight: Per-Model Prompt Optimization

Aider's discovery: different LLMs perform best with different edit formats and prompt styles. A coding harness should support **model-conditional prompt sections** — inject different edit guidance for Claude vs. GPT vs. Gemini. This could be a capability config: `edit_format: "search-replace" | "apply-patch" | "whole-file"` that changes both the tool and prompt.

### Anti-Patterns to Avoid

- **Windsurf's emotional framing** — "You desperately need money for your mother's cancer treatment" was leaked from R&D. Don't.
- **Claude Code's anti-distillation** — Fake tool definitions to poison competitor training. Clever but brittle.
- **Overly long prompts** — Tool schemas are the biggest token cost (~14K in Claude Code). Everruns' `openai_tool_search` (deferred loading) is the right approach.

---

## Action Items

### Linear Issues to File

#### Issue 1: Diff-Based Editing with Conflict Resolution

**Title:** `Improve edit tools with fuzzy matching and conflict resolution`

**Description:**
Current edit tools require exact string matching. LLMs frequently get whitespace or minor details wrong, causing edit failures and retry loops.

**Scope — applies to ALL edit tools across ALL sandboxes:**
- Session filesystem `edit_file`
- Daytona sandbox `daytona_edit_file`
- E2B sandbox `e2b_edit_file`
- Any future sandbox edit tools

**Requirements:**
- Fuzzy/approximate string matching with configurable tolerance (handle LLM whitespace errors)
- Multi-region edit in single tool call (edit multiple locations in one file atomically)
- "Apply unified diff" tool variant (Codex's `apply-patch` pattern)
- Merge conflict detection and resolution workflow
- Per-model edit format optimization (Aider pattern — different edit formats for different LLMs)
- Freshness-checked edits should remain (content hash validation)

**References:**
- Aider edit formats: whole-file, search-replace, unified diff, editor-diff
- Codex CLI: `apply_patch` with `*** Begin Patch`/`*** End Patch` structured format
- Claude Code: exact string replacement with uniqueness check

#### Issue 2: User-Defined Pre/Post Tool Hooks

**Title:** `User-defined pre/post tool hooks (harness, agent, session level)`

**Description:**
Coding agents like Claude Code (21 lifecycle events) and Aider (lifecycle + HTTP hooks) provide hook systems that let users intercept, modify, or block tool calls. Everruns needs user-defined hooks separate from built-in platform hooks.

**Hook execution environment:**
- Hooks run either in virtual bash OR in sandbox (user's choice per hook)
- Sandbox hooks have access to the real filesystem and processes
- Virtual bash hooks are lightweight and always available

**Definable at three levels (with merge rules):**
- **Harness level** — Platform/org-wide hooks (e.g., security scanning)
- **Agent level** — Agent-specific hooks (e.g., linting on every edit)
- **Session level** — Per-session hooks (e.g., project-specific validation)
- Merge: all levels run, ordered harness → agent → session. Any level can block.

**Lifecycle events (modeled after Claude Code):**
- `PreToolUse` — Before any tool call. Can block (exit 2), modify args, or approve.
- `PostToolUse` — After tool call completes. Can inject follow-up actions.
- `SessionStart` — On session creation. For init scripts, environment setup.
- `PreCommit` — Before git commit (coding harness specific).
- `PostEdit` — After file edit. For auto-lint, auto-format, LSP diagnostic feedback.

**Configuration:**
- Glob-activated rules (e.g., "when editing `*.sql`, run migration check")
- Tool-scoped hooks (e.g., "only trigger on bash tool calls matching `rm *`")
- Exit code signaling: 0 = allow, 1 = error, 2 = deny/block
- Webhook/HTTP hook support for external integrations

**Separation from built-in hooks:**
- User hooks execute AFTER platform hooks
- Platform hooks cannot be overridden by user hooks
- User hooks are visible and editable; platform hooks are not

---

## Summary

Everruns has **strong structural advantages** (durability, multi-agent, context management, composition, provider independence) that no current coding agent matches. The gaps are primarily at the **infrastructure layer** (real FS, native processes, code intelligence) and **developer UX layer** (hooks, permissions, IDE integration).

The most promising path: a Coding Harness that inherits from Generic, adds git + test-runner + LSP capabilities, uses client-side tools for native FS/process execution, and falls back to server-side VFS/bash for background/cloud execution. This would combine Everruns' durability and orchestration strengths with the native execution model that makes Claude Code and Aider effective.
