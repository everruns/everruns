---
type: Specification
title: "Agent Instructions Specification"
description: "AGENTS.md support (dynamic project instructions)."
tags:
  - everruns
  - runtime-resources
---
# Agent Instructions Specification

## Abstract

The agent instructions capability resolves configured Markdown instruction files hierarchically — from the session filesystem root down to the working directory — and injects them as the leading user-role message on every LLM turn. By default it reads `AGENTS.md`, preserving Everruns' original AGENTS.md behavior, while allowing agents to opt into files such as `CLAUDE.md` at every hierarchy level. Workspace files are untrusted third-party content and never enter the system prompt.

## Background

`AGENTS.md` is an emerging open standard (backed by OpenAI, Google, Cursor, Sourcegraph, Linux Foundation) for providing project-level instructions to AI coding agents. There is no formal specification beyond "plain markdown in a file named AGENTS.md." Each tool defines its own discovery and injection semantics.

Everruns implements AGENTS.md as a built-in capability that reads from the session workspace filesystem. This integrates naturally with the existing capability system, users enable/disable it per agent, configure optional additional file names per capability assignment, and the content is picked up dynamically (no restart needed).

## Design Decisions

| Question | Decision |
|----------|----------|
| File name | Defaults to `AGENTS.md`; configurable `files` list can include names such as `CLAUDE.md` |
| Discovery | Configured filenames resolved hierarchically from the filesystem root down to the working directory (`resolve_path(".")` anchor); deeper files override shallower ones, sibling subtrees out of scope |
| Injection point | Leading user-role message of every turn (conversation context) — never the system prompt; system instructions always take precedence |
| Dynamic reading | Re-read on every LLM turn (picks up changes immediately) |
| Size limit | 32 KiB max per instruction file (truncated with warning, excluding wrapper/hint text, matching Codex convention) plus a 128 KiB total per-turn budget binding hierarchy depth (deeper files past the budget omitted with an explicit note) |
| Missing file | Silently ignored per configured file (no error) |
| Format | Plain markdown, no special syntax, no `@` imports |
| Link-following hint | Appended after content; nudges LLM to read referenced files progressively |
| Architecture | Self-contained capability with `conversation_context_contribution()` override; the turn loop inserts the collected block at the head of the per-request messages |
| Dependencies | None required; `session_file_system` recommended for authoring |

## Authoring Guidance

Repository `AGENTS.md` files should stay short enough to be useful as hot prompt context. Keep required workflow, git/PR rules, and local commands in the instruction file; move durable maps, catalogs, and long process detail into the knowledge bundle or other referenced docs. The Everruns root `AGENTS.md` should target roughly 2-5 KiB and point agents to `knowledge/index.md` for the knowledge index.

## Capability Definition

The capability encapsulates all AGENTS.md logic: hierarchical discovery from the session filesystem,
formatting, size limiting, and XML wrapping. It uses config-aware `conversation_context_contribution()`
async method (via `SystemPromptContext`) to access the session filesystem. Its `system_prompt_contribution()`
intentionally returns `None`: untrusted workspace content must stay below harness safety instructions
in the instruction hierarchy and out of the cache-stable system prefix.

See `crates/builtins/src/agent_instructions.rs` for the `AgentInstructionsCapability` implementation.

## SystemPromptContext

Capabilities that need dynamic system prompt content receive a `SystemPromptContext`
with access to session-specific resources. See `crates/core/src/capabilities/mod.rs` for the `SystemPromptContext` struct.

The context is constructed in `ReasonAtom` and passed through the async builder
methods (`with_harness_async`, `with_agent_async`, `with_capabilities_async`).

## Integration Flow

```
execute_llm_call()
  ├── Load agent + session
  ├── Create SystemPromptContext { session_id, file_store }
  ├── Build RuntimeAgent using async builder methods
  │   ├── with_harness_async() — resolves harness capabilities (including agent_instructions)
  │   ├── with_agent_async() — resolves agent capabilities
  │   └── with_capabilities_async() — resolves session capabilities
  │       └── For each capability: call system_prompt_contribution(ctx)
  │           └── agent_instructions: reads configured instruction files from file store
  ├── Build final RuntimeAgent
  └── Execute LLM call
```

### Turn message order

Every turn the model sees, top to bottom:

1. **System prompt**: harness safety instructions, capability additions, agent base prompt (cache-stable)
2. **Conversation context**: the resolved hierarchy behind a trust framing header, each file wrapped in `<agent-instructions source="...">` tags, broadest scope first
3. **Conversation history and the latest user message**

XML tags provide clear boundaries between sections. See `knowledge/project/xml-prompt-formatting.md` for rationale.

## ReasonAtom Changes

ReasonAtom holds an optional `SessionFileSystem` that is passed to capabilities via `SystemPromptContext`. See `crates/engine/src/execution/reason.rs` for the `with_file_store` builder method.

## Constants

See `crates/builtins/src/agent_instructions.rs` for `MAX_AGENTS_MD_SIZE` (32 KiB), `MAX_TOTAL_AGENT_INSTRUCTIONS_BYTES` (128 KiB), `AGENTS_MD_PATH`, `DEFAULT_AGENT_INSTRUCTIONS_FILE`, `MAX_AGENT_INSTRUCTIONS_FILES`, and `AGENT_INSTRUCTIONS_CAPABILITY_ID`.

## API

The capability appears in standard capability endpoints:

```http
GET /v1/capabilities

Response includes:
{
  "id": "agent_instructions",
  "name": "AGENTS.md",
  "description": "Resolves AGENTS.md hierarchies as conversation context...",
  "status": "available",
  "icon": "file-text",
  "category": "Configuration",
  "config_schema": {
    "properties": {
      "files": {
        "default": ["AGENTS.md"]
      }
    }
  }
}
```

Enable on an agent:

```http
PATCH /v1/agents/{id}
{ "capabilities": [{ "ref": "agent_instructions" }, ...] }
```

Configure additional instruction files:

```http
PATCH /v1/agents/{id}
{
  "capabilities": [
    {
      "ref": "agent_instructions",
      "config": {
        "files": ["AGENTS.md", "CLAUDE.md"]
      }
    }
  ]
}
```

## Usage

1. Enable the `agent_instructions` capability on an agent
2. Write an `AGENTS.md` file to the session workspace (via file tools, bash, or API)
3. Optionally configure `files` when the agent should also read another instruction file
4. The agent automatically reads configured files on every turn
5. Edit a configured file anytime, changes take effect on the next turn
