# Coding Daytona Harness

Built-in harness for coding agents using Daytona cloud sandboxes.

## Design

**Name:** `coding-daytona`
**Display Name:** Coding (Daytona)
**Parent:** `generic` (inherits session_file_system, virtual_bash, web_fetch, session_storage, session, agent_instructions, skills, infinity_context, openai_tool_search, budgeting, compaction, tool_output_persistence)
**Additional capability:** `daytona`
**Roles:** None (not Base, Default, or Chat — opt-in harness)

## Architecture: Two-Level Execution

The harness operates at two levels:

1. **Workspace (VFS + virtual bash)** — inherited from Generic. For configuration, notes, artifacts, lightweight operations.
2. **Daytona sandbox** — real filesystem, real processes, real network. For actual coding: git clone, builds, tests, dev servers.

The system prompt steers the LLM to use the right level for each task.

## System Prompt Design

The system prompt follows patterns from state-of-the-art coding agents (Claude Code, Codex, Aider, Cursor). Key sections:

1. **Identity** — Expert software developer with sandbox access
2. **Tool selection steering** — When to use sandbox tools vs. workspace tools (highest-impact section)
3. **Coding workflow** — Edit-test-fix loop as the primary pattern
4. **Code quality guardrails** — Don't over-engineer, read before edit, minimal changes
5. **Git safety** — Never force push, conventional commits, read errors before retrying
6. **Error recovery** — Read errors, don't blind-retry, fix root cause
7. **Output format** — `file:line` references, concise, no tool name leaks

## Capability Stack (Effective)

Inherited from Generic:
- `session_file_system` — workspace VFS
- `virtual_bash` — lightweight sandboxed shell
- `web_fetch` (with `enable_file_download: true`)
- `session_storage` — KV store + encrypted secrets
- `session` — session metadata
- `agent_instructions` — AGENTS.md support
- `skills` — skill discovery
- `infinity_context` — long conversation support
- `openai_tool_search` — deferred tool loading
- `budgeting` — budget awareness
- `compaction` (auto, proactive, 85%) — context management
- `tool_output_persistence` — full output capture

Added by this harness:
- `daytona` — cloud sandbox (file read/write/edit, bash exec, git clone, git credentials, snapshots, lifecycle management)

## Coding Workflow

```
1. Create sandbox (daytona_create_sandbox)
2. Clone repo (daytona_git_clone)
3. Read code (daytona_read_file)
4. Edit code (daytona_exec: sed/patch, or daytona_write_file)
5. Run tests (daytona_exec)
6. See failures → fix → re-run (loop)
7. Commit & push (daytona_exec + daytona_git_credentials)
8. Delete sandbox (daytona_manage_sandbox)
```

## What This Harness Does NOT Include

- **LSP integration** — Not yet available. Explore subagent-based code exploration first.
- **IDE integration** — Separate product surface.
- **Permission filtering** — Operating in YOLO mode. Future work.
- **Pre/post tool hooks** — User-defined hooks are a separate feature (see Linear backlog).
- **Per-model edit format optimization** — Future enhancement.

## Implementation

See `crates/server/src/platform.rs` for harness definition.
See `crates/server/src/org_init.rs` for seed ID.
