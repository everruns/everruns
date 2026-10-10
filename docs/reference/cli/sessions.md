---
title: everruns sessions
description: "Running and archived sessions, their state and participants. CLI reference for everruns sessions."
sidebar:
  label: sessions
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Running and archived sessions, their state and participants.

| Command | What it does |
|---|---|
| [`sessions create`](#sessions-create) | Create a new session. |
| [`sessions watch`](#sessions-watch) | Watch session events in real time. |
| [`sessions export`](#sessions-export) | Export session messages as JSONL or an ATIF trajectory. |
| [`sessions archive`](#sessions-archive) | Archive a session so it drops out of default lists. |
| [`sessions cancel`](#sessions-cancel) | Cancel the currently executing turn in a session. |
| [`sessions delete`](#sessions-delete) | Delete a session. |
| [`sessions fork`](#sessions-fork) | Fork a session into a new, independent session that copies its conversation history and workspace files. |
| [`sessions get`](#sessions-get) | Get session details including status, agent, harness, and model. |
| [`sessions context`](#sessions-context) | Get the latest estimated context token breakdown for a session, grouped by system prompt, tools, rules, skills, MCP, subagents, and conversation. |
| [`sessions facets`](#sessions-facets) | Counts per status, source, and agent plus masthead metrics for the sessions list, over the same filters as list_sessions. |
| [`sessions stats`](#sessions-stats) | Get session counts by status. |
| [`sessions list`](#sessions-list) | List sessions. |
| [`sessions pin`](#sessions-pin) | Pin a session for the current user. |
| [`sessions resume`](#sessions-resume) | Resume all paused session budgets for a session. |
| [`sessions unarchive`](#sessions-unarchive) | Restore an archived session to default lists. |
| [`sessions unpin`](#sessions-unpin) | Unpin a session for the current user. |
| [`sessions update`](#sessions-update) | Update session title, tags, or locale. |
| [`sessions budget-check check`](#sessions-budget-check-check) | Check all budgets for a session. |
| [`sessions budgets list`](#sessions-budgets-list) | List all budgets for a session. |
| [`sessions databases create`](#sessions-databases-create) | Create a new SQL database inside a session. |
| [`sessions databases delete`](#sessions-databases-delete) | Delete a session SQL database. |
| [`sessions databases get`](#sessions-databases-get) | Get metadata for a session SQL database. |
| [`sessions databases list`](#sessions-databases-list) | List all SQL databases created inside a session. |
| [`sessions databases schema get`](#sessions-databases-schema-get) | Inspect the schema of a session SQL database. |
| [`sessions events list`](#sessions-events-list) | List events for a session. |
| [`sessions events summary events`](#sessions-events-summary-events) | One-shot debug summary for a session: counts by type, first/last timestamps, turn count, error count. |
| [`sessions fs create`](#sessions-fs-create) | Create a file or directory in the session filesystem. |
| [`sessions fs delete`](#sessions-fs-delete) | Delete a file or directory in the session filesystem. |
| [`sessions fs get`](#sessions-fs-get) | Get a file or directory at a path in the session filesystem. |
| [`sessions fs list`](#sessions-fs-list) | Get the root directory listing of session files. |
| [`sessions fs update`](#sessions-fs-update) | Update a file in the session filesystem. |
| [`sessions fs - copy`](#sessions-fs---copy) | Copy a file in the session filesystem. |
| [`sessions fs - grep`](#sessions-fs---grep) | Search files in the session filesystem. |
| [`sessions fs - move`](#sessions-fs---move) | Move or rename a file in the session filesystem. |
| [`sessions fs - search`](#sessions-fs---search) | Search files in the session filesystem, with surrounding context and paging. |
| [`sessions fs - stat`](#sessions-fs---stat) | Get file metadata in the session filesystem. |
| [`sessions mcp-servers list`](#sessions-mcp-servers-list) | List the MCP servers added to one chat only. |
| [`sessions mcp-servers remove`](#sessions-mcp-servers-remove) | Remove an MCP server added to one chat only. |
| [`sessions messages create`](#sessions-messages-create) | Create a user message in a session and start the next run. |
| [`sessions messages ratings`](#sessions-messages-ratings) | List your good and bad ratings of the messages in a session. |
| [`sessions messages list`](#sessions-messages-list) | List materialized messages in a session, optionally limited to the most recent N. |
| [`sessions messages rate`](#sessions-messages-rate) | Rate one message in a session good or bad, with an optional comment. |
| [`sessions participants add`](#sessions-participants-add) | Add a member participant to a session. |
| [`sessions participants leave`](#sessions-participants-leave) | Mark a session member participant as having left. |
| [`sessions participants list`](#sessions-participants-list) | List the participant history for a session. |
| [`sessions platform-chat ensure`](#sessions-platform-chat-ensure) | Open the current user's permanent platform conversation. |
| [`sessions resources list`](#sessions-resources-list) | List all resources registered in a session. |
| [`sessions sandbox get`](#sessions-sandbox-get) | Inspect a Session's primary Sandbox and what it may touch. |
| [`sessions sandbox manage`](#sessions-sandbox-manage) | Pause, resume, or delete the managed sandbox for a session. |
| [`sessions sse stream`](#sessions-sse-stream) | Stream events via SSE. |
| [`sessions storage keys list`](#sessions-storage-keys-list) | List all key-value pairs stored for a session. |
| [`sessions storage secrets batch`](#sessions-storage-secrets-batch) | Encrypt and store multiple session secrets in one request. |
| [`sessions storage secrets delete`](#sessions-storage-secrets-delete) | Delete a user-managed encrypted session secret by name. |
| [`sessions storage secrets list`](#sessions-storage-secrets-list) | List all secrets stored for a session without revealing values. |
| [`sessions tasks cancel`](#sessions-tasks-cancel) | Request cooperative cancellation of a session task. |
| [`sessions tasks get`](#sessions-tasks-get) | Get one session task with its recent message thread. |
| [`sessions tasks list`](#sessions-tasks-list) | List background tasks owned by a session. |
| [`sessions tasks messages post`](#sessions-tasks-messages-post) | Send an inbound message to a session task. |
| [`sessions tasks push-configs create`](#sessions-tasks-push-configs-create) | Create a per-task push-notification config. |
| [`sessions tasks push-configs delete`](#sessions-tasks-push-configs-delete) | Delete a per-task push-notification config. |
| [`sessions tasks push-configs list`](#sessions-tasks-push-configs-list) | List per-task push-notification configs. |
| [`sessions tool-results submit`](#sessions-tool-results-submit) | Submit client-side tool results back to a waiting session. |
| [`sessions trace get`](#sessions-trace-get) | Summarize a session's trace: turn, step and error totals, a minimap of turn buckets, and the turns that failed. |
| [`sessions trace turns list`](#sessions-trace-turns-list) | List a page of a session's turns with their steps: model calls, tool calls, approvals, sub-agents and messages, with repeated calls folded into batches. |
| [`sessions trace turns events list`](#sessions-trace-turns-events-list) | List the raw events of one turn of a session's trace, without deltas; small payloads are inlined. |
| [`sessions trace turns steps get`](#sessions-trace-turns-steps-get) | Read one step of a session's trace in full: tool input and output, the model request and response, and its raw events. |
| [`sessions trace turns steps list`](#sessions-trace-turns-steps-list) | Page through the steps of one turn of a session's trace, optionally only the failed ones. |
| [`sessions trace turns steps request list`](#sessions-trace-turns-steps-request-list) | Page through the messages a model call in a session's trace was sent, with each message's full content. |

## sessions create

Create a new session.

```bash
everruns sessions create [OPTIONS]
```

| Flag | Description |
|---|---|
| `-H`, `--harness <HARNESS>` | Harness ID or name (e.g. harness_xxx or "generic"). Omit to derive from the agent (when given), else the org default. |
| `-a`, `--agent <AGENT>` | Agent ID or name (optional, e.g. agent_xxx or "support"). When set without --harness, the session runs on the agent's harness. |
| `--title <TITLE>` | Session title. |
| `--locale <LOCALE>` | Session locale (BCP 47, e.g. uk-UA) |
| `--model <MODEL>` | Model ID override (e.g. mod_xxx) |
| `--virtual-user <VIRTUAL_USER>` | Resident virtual user ID for unattended/background execution. |
| `--system-prompt <SYSTEM_PROMPT>` | Session-level system prompt override. |
| `-t`, `--tag <TAGS>` | Session tag (repeatable) Repeatable. |
| `--capability <REF[=JSON]>` | Session capability (repeatable). Format: REF or REF=JSON_CONFIG. Repeatable. |
| `--hint <KEY=JSON>` | Session client hint (repeatable). Format: KEY=JSON_VALUE. Repeatable. |
| `--hints-json <JSON>` | Session client hints JSON object. |
| `--network-allow <PATTERN>` | Network allow pattern (repeatable) Repeatable. |
| `--network-block <PATTERN>` | Network block pattern (repeatable) Repeatable. |
| `--max-iterations <MAX_ITERATIONS>` | Maximum LLM iterations per turn. |
| `--secret <KEY=VALUE>` | Session-scoped secret (repeatable, format: KEY=VALUE) Repeatable. |
| `--budget-limit <[CURRENCY:]LIMIT>` | Budget limit (repeatable). Format: [CURRENCY:]LIMIT. Currency defaults to usd. Examples: --budget-limit 10 ($10 USD) --budget-limit usd:10 ($10 USD, explicit) --budget-limit tokens:2000000 (2M token limit) Multiple limits stack — most restrictive wins. Repeatable. |
| `--budget-soft-limit <[CURRENCY:]LIMIT>` | Budget soft limit — pauses before hard stop. Same format as --budget-limit. Must pair with a --budget-limit of the same currency. Repeatable. |


## sessions watch

Watch session events in real time.

```bash
everruns sessions watch <SESSION>
```

| Flag | Description |
|---|---|
| `<SESSION>` | Required. Session ID (e.g. ses_xxx) |


## sessions export

Export session messages as JSONL or an ATIF trajectory.

```bash
everruns sessions export [OPTIONS] <SESSION>
```

| Flag | Description |
|---|---|
| `<SESSION>` | Required. Session ID (e.g. session_xxx) |
| `--out <OUT>` | File to write (defaults to stdout). Not `-o/--output`: that is the global output-format flag, and sharing its id made clap fill this with the format's default, so every export wrote a file named `text`. |
| `--format <FORMAT>` | Export format: `jsonl` (one message per line, default) or `atif` (a single ATIF trajectory JSON document) One of `jsonl`, `atif`. |


## sessions archive

Archive a session so it drops out of default lists.

```bash
everruns sessions archive [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Move a finished session out of the active list
everruns sessions archive session_01h9 --reason 'Release shipped'
```

## sessions cancel

Cancel the currently executing turn in a session.

```bash
everruns sessions cancel [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Stop a session that is running away
everruns sessions cancel session_01h9
```

## sessions delete

Delete a session.

```bash
everruns sessions delete [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Archive a session, keeping it restorable
everruns sessions delete session_01h9 --reason 'Duplicate of the release session'
```

## sessions fork

Fork a session into a new, independent session that copies its conversation history and workspace files.

```bash
everruns sessions fork [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session to fork (prefixed public id). |
| `--overrides <OVERRIDES>` | Request to fork a session. |

Example:

```bash
# Branch from a session to try a different direction
everruns sessions fork session_01h9 --reason 'Try the rollback path'
```

## sessions get

Get session details including status, agent, harness, and model.

```bash
everruns sessions get [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Show one session's state and configuration
everruns sessions get session_01h9
```

## sessions context

Get the latest estimated context token breakdown for a session, grouped by system prompt, tools, rules, skills, MCP, subagents, and conversation.

```bash
everruns sessions context [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# See what is filling a session's context window
everruns sessions context session_01h9
```

## sessions facets

Counts per status, source, and agent plus masthead metrics for the sessions list, over the same filters as list_sessions.

```bash
everruns sessions facets [OPTIONS]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` |  |
| `--archived-only` | Return only archived sessions. |
| `--created-after <CREATED_AFTER>` | Inclusive lower bound on `created_at` (RFC 3339). |
| `--created-before <CREATED_BEFORE>` | Exclusive upper bound on `created_at` (RFC 3339). |
| `--include-archived` | Include archived sessions. |
| `--mine` | Restrict to sessions owned by the calling user. |
| `--order <ORDER>` | `created_at` (default) or `last_activity`. |
| `--playground-user-id <PLAYGROUND_USER_ID>` |  |
| `--search <SEARCH>` | Case-insensitive title substring match. |
| `--side-chats-only` | Exclude the permanent Chat from side-conversation pagination. |
| `--source <SOURCE>` | Comma-separated sources (`chat`, `api`, `slack`, `ag_ui`, `fcp`, `schedule`, `webhook`, `a2a`... |
| `--status <STATUS>` | Comma-separated derived activities (`running`, `paused`, `failed`, `completed`, `idle`). |

Example:

```bash
# Break the session list down by status, agent and source
everruns sessions facets --search triage
```

## sessions stats

Get session counts by status.

```bash
everruns sessions stats [OPTIONS]
```

Example:

```bash
# Check token and cost totals across sessions
everruns sessions stats
```

## sessions list

List sessions. Filter by agent_id, source, status, owner (mine), and creation window; search by title; order by created_at or last_activity. Supports pagination (limit/offset).

```bash
everruns sessions list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` |  |
| `--archived-only` | Return only archived sessions. |
| `--created-after <CREATED_AFTER>` | Inclusive lower bound on `created_at` (RFC 3339). |
| `--created-before <CREATED_BEFORE>` | Exclusive upper bound on `created_at` (RFC 3339). |
| `--include-archived` | Include archived sessions. |
| `--limit <LIMIT>` | Maximum number of items returned in this page. |
| `--mine` | Restrict to sessions owned by the calling user. |
| `--offset <OFFSET>` | Zero-based offset into the result set. |
| `--order <ORDER>` | `created_at` (default) or `last_activity`. |
| `--playground-user-id <PLAYGROUND_USER_ID>` |  |
| `--search <SEARCH>` | Case-insensitive title substring match. |
| `--side-chats-only` | Exclude the permanent Chat from side-conversation pagination. |
| `--source <SOURCE>` | Comma-separated sources (`chat`, `api`, `slack`, `ag_ui`, `fcp`, `schedule`, `webhook`, `a2a`... |
| `--status <STATUS>` | Comma-separated derived activities (`running`, `paused`, `failed`, `completed`, `idle`). |

Example:

```bash
# Find recent sessions when you do not know the id
everruns sessions list --limit 20
```

## sessions pin

Pin a session for the current user.

```bash
everruns sessions pin [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Keep a session at the top of the list
everruns sessions pin session_01h9 --reason 'Active incident'
```

## sessions resume

Resume all paused session budgets for a session.

```bash
everruns sessions resume [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Restart a session that paused when a budget ran out
everruns sessions resume session_01h9 --reason 'Budget topped up'
```

## sessions unarchive

Restore an archived session to default lists.

```bash
everruns sessions unarchive [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Bring an archived session back to the active list
everruns sessions unarchive session_01h9 --reason 'Release reopened'
```

## sessions unpin

Unpin a session for the current user.

```bash
everruns sessions unpin [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Stop keeping a session at the top of the list
everruns sessions unpin session_01h9 --reason 'Incident resolved'
```

## sessions update

Update session title, tags, or locale.

```bash
everruns sessions update [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |
| `--goal <GOAL>` | Updated session objective. |
| `--locale <LOCALE>` | Session locale (BCP 47, e.g. |
| `--tags <TAGS>` | Tags for organizing and filtering sessions. Repeatable. |
| `--title <TITLE>` | Human-readable title for the session. |
| `--virtual-user-id <VIRTUAL_USER_ID>` | Optional resident virtual user used for unattended/background execution. |

Example:

```bash
# Retitle a session so it is findable later
everruns sessions update session_01h9 --title 'Release triage' --reason 'Clarify the session topic'
```

## sessions budget-check check

Check all budgets for a session.

```bash
everruns sessions budget-check check [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# See if any budget would stop this session from running
everruns sessions budget-check check session_01h9
```

## sessions budgets list

List all budgets for a session.

```bash
everruns sessions budgets list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# List every budget that constrains a session
everruns sessions budgets list session_01h9
```

## sessions databases create

Create a new SQL database inside a session.

```bash
everruns sessions databases create [OPTIONS] --name <name> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Required. Human-readable name. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Give a session a scratch SQL database to work with
everruns sessions databases create --session-id session_01h9 --name analytics --reason 'Store intermediate results'
```

## sessions databases delete

Delete a session SQL database.

```bash
everruns sessions databases delete [OPTIONS] --name <name> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Required. Human-readable name. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Drop a session database once its data is exported
everruns sessions databases delete --session-id session_01h9 --name analytics --reason 'No longer needed'
```

## sessions databases get

Get metadata for a session SQL database.

```bash
everruns sessions databases get [OPTIONS] --name <name> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Required. Human-readable name. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Check a session database's size and metadata
everruns sessions databases get --session-id session_01h9 --name analytics
```

## sessions databases list

List all SQL databases created inside a session.

```bash
everruns sessions databases list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# See which databases a session has created
everruns sessions databases list session_01h9
```

## sessions databases schema get

Inspect the schema of a session SQL database.

```bash
everruns sessions databases schema get [OPTIONS] --name <name> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Required. Human-readable name. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Inspect a session database's tables before writing queries
everruns sessions databases schema get --session-id session_01h9 --name analytics
```

## sessions events list

List events for a session.

```bash
everruns sessions events list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |
| `--after-sequence <AFTER_SEQUENCE>` | Forward cursor: only return events with sequence > after_sequence. Mutually exclusive with `b... |
| `--around <AROUND>` |  |
| `--before-sequence <BEFORE_SEQUENCE>` | Backward cursor: only events with sequence < before_sequence. |
| `--exclude <EXCLUDE>` | Omit these event types. Repeatable. |
| `--exec-id <EXEC_ID>` | Filter by `context.exec_id`. |
| `--from-ts <FROM_TS>` | `created_at >= from_ts` (RFC 3339). |
| `--limit <LIMIT>` | Maximum number of items returned in this page. |
| `--order-desc` | When true, return newest first; default oldest first. |
| `--q <Q>` | Full-text search via Postgres tsvector (substring fallback in-memory). |
| `--since-id <SINCE_ID>` |  |
| `--tags <TAGS>` | Tag any-match against `events.tags`. Repeatable. |
| `--to-ts <TO_TS>` | `created_at <= to_ts` (RFC 3339). |
| `--tool-name <TOOL_NAME>` | Match `data.tool_name` (useful for `tool.*` events). |
| `--trace-id <TRACE_ID>` | Filter by `context.trace_id`. |
| `--turn-id <TURN_ID>` | Filter by `context.turn_id`. |
| `--types <TYPES>` | Only these event types, e.g. Repeatable. |
| `--window <WINDOW>` | Window size for `around` (events on each side). |

Example:

```bash
# Find what failed in a session's tool calls
everruns sessions events list session_01h9 --types tool.failed

# Search a session's events for a phrase, newest first
everruns sessions events list session_01h9 --q timeout --order-desc true
```

## sessions events summary events

One-shot debug summary for a session: counts by type, first/last timestamps, turn count, error count.

```bash
everruns sessions events summary events [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Get a quick picture of a session (event counts, turns, errors) before reading events
everruns sessions events summary events session_01h9
```

## sessions fs create

Create a file or directory in the session filesystem.

```bash
everruns sessions fs create [OPTIONS] --path <path> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--content <CONTENT>` | File content (text or base64-encoded). |
| `--encoding <ENCODING>` | Content encoding: "text" or "base64". |
| `--is-directory` | Whether to create a directory instead of a file (ignores `content`/`encoding`). |
| `--is-readonly` | Whether file is read-only. |
| `--path <PATH>` | Required. Path in the session filesystem (relative to the filesystem root). |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Seed a session with a file the agent should read
everruns sessions fs create --session-id session_01h9 --path /brief.md --content '# Brief' --reason 'Give the agent context'
```

## sessions fs delete

Delete a file or directory in the session filesystem.

```bash
everruns sessions fs delete [OPTIONS] --path <path> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--path <PATH>` | Required. Path in the session filesystem (relative to the filesystem root). |
| `--recursive` | Delete a directory and everything under it. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Remove a scratch directory from a session's filesystem
everruns sessions fs delete --session-id session_01h9 --path /tmp --recursive true --reason 'Clean up scratch files'
```

## sessions fs get

Get a file or directory at a path in the session filesystem.

```bash
everruns sessions fs get [OPTIONS] --path <path> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--path <PATH>` | Required. Path in the session filesystem (relative to the filesystem root). |
| `--recursive` | List nested entries recursively when the path is a directory. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Read a file the agent wrote in a session
everruns sessions fs get --session-id session_01h9 --path /report.md
```

## sessions fs list

Get the root directory listing of session files.

```bash
everruns sessions fs list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |
| `--recursive` | List nested entries recursively, not just the top level. |

Example:

```bash
# See what files a session contains
everruns sessions fs list session_01h9
```

## sessions fs update

Update a file in the session filesystem.

```bash
everruns sessions fs update [OPTIONS] --path <path> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--content <CONTENT>` | New file content. |
| `--encoding <ENCODING>` | Content encoding: "text" or "base64". |
| `--expected-content <EXPECTED_CONTENT>` | Content the file must currently hold for the write to happen. When set, the update is a comp... |
| `--expected-encoding <EXPECTED_ENCODING>` | Encoding of `expected_content`. |
| `--is-readonly` | Whether file is read-only. |
| `--path <PATH>` | Required. Path in the session filesystem (relative to the filesystem root). |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Overwrite a file in a session's filesystem
everruns sessions fs update --session-id session_01h9 --path /brief.md --content '# Updated brief' --reason 'Correct the scope'
```

## sessions fs - copy

Copy a file in the session filesystem.

```bash
everruns sessions fs - copy [OPTIONS] --dst-path <dst_path> --session-id <session_id> --src-path <src_path>
```

| Flag | Description |
|---|---|
| `--dst-path <DST_PATH>` | Required. Destination path (relative to the workspace filesystem root). |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--src-path <SRC_PATH>` | Required. Source path (relative to the workspace filesystem root). |

Example:

```bash
# Duplicate a file in a session's filesystem before editing it
everruns sessions fs - copy --session-id session_01h9 --src-path /notes.md --dst-path /notes.bak.md --reason 'Keep a backup'
```

## sessions fs - grep

Search files in the session filesystem.

```bash
everruns sessions fs - grep [OPTIONS] --pattern <pattern> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--path-pattern <PATH_PATTERN>` | Optional path glob to filter files (`**/*.rs`, `docs/*.md`). |
| `--pattern <PATTERN>` | Required. Regex pattern to search for. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Find which files in a session mention a word
everruns sessions fs - grep --session-id session_01h9 --pattern TODO
```

## sessions fs - move

Move or rename a file in the session filesystem.

```bash
everruns sessions fs - move [OPTIONS] --dst-path <dst_path> --session-id <session_id> --src-path <src_path>
```

| Flag | Description |
|---|---|
| `--dst-path <DST_PATH>` | Required. Destination path (relative to the workspace filesystem root). |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--src-path <SRC_PATH>` | Required. Source path (relative to the workspace filesystem root). |

Example:

```bash
# Rename a file in a session's filesystem
everruns sessions fs - move --session-id session_01h9 --src-path /draft.md --dst-path /final.md --reason 'Publish the draft'
```

## sessions fs - search

Search files in the session filesystem, with surrounding context and paging.

```bash
everruns sessions fs - search [OPTIONS] --pattern <pattern> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--after-context <AFTER_CONTEXT>` | Lines of context to return after each match. |
| `--before-context <BEFORE_CONTEXT>` | Lines of context to return before each match. |
| `--limit <LIMIT>` | Maximum matches to return. |
| `--max-bytes <MAX_BYTES>` | Byte ceiling on the returned payload. |
| `--offset <OFFSET>` | Number of matches to skip, for paging through a large result set. |
| `--path-pattern <PATH_PATTERN>` | Glob limiting which paths are searched. |
| `--pattern <PATTERN>` | Required. Regular expression to match against file contents. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Search file contents with surrounding lines
everruns sessions fs - search --session-id session_01h9 --pattern 'TODO|FIXME' --path-pattern '**/*.rs' --after-context 2
```

## sessions fs - stat

Get file metadata in the session filesystem.

```bash
everruns sessions fs - stat [OPTIONS] --path <path> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--path <PATH>` | Required. Path to the file or directory (relative to the workspace filesystem root). |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Check a file's size and type without reading it
everruns sessions fs - stat --session-id session_01h9 --path /report.md
```

## sessions mcp-servers list

List the MCP servers added to one chat only.

```bash
everruns sessions mcp-servers list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# See which MCP servers were added to one chat only
everruns sessions mcp-servers list session_01h9
```

## sessions mcp-servers remove

Remove an MCP server added to one chat only. Its tools leave from the next turn.

```bash
everruns sessions mcp-servers remove [OPTIONS] --name <name> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Required. Server name. |
| `--session-id <SESSION_ID>` | Required. Session the server was added to. |

Example:

```bash
# Drop a chat-only MCP server so its tools leave from the next turn
everruns sessions mcp-servers remove --session-id session_01h9 --name github --reason 'No longer needed in this chat'
```

## sessions messages create

Create a user message in a session and start the next run. The message content is an array of content parts, e.g. --content '[{"type":"text","text":"Tell me a short, family-friendly joke."}]'.

```bash
everruns sessions messages create [OPTIONS] --message <message> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--addressed-participant-id <ADDRESSED_PARTICIPANT_ID>` |  |
| `--client-message-id <CLIENT_MESSAGE_ID>` | Client-minted id that makes a retried send idempotent. |
| `--controls <CONTROLS>` |  |
| `--external-actor <EXTERNAL_ACTOR>` |  |
| `--message <MESSAGE>` | Required. Input message for creating a user message Only user messages can be created via the API. Age... |
| `--metadata <METADATA>` | Free-form metadata attached to this resource. |
| `--request-id <REQUEST_ID>` | Caller-chosen request id, carried through to the run and logs for correlation. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--tags <TAGS>` | Free-form tags attached to this resource. Repeatable. |

Example:

```bash
# Send a user message to a session and start the next run
everruns sessions messages create --session-id session_01h9 --message '{"content":[{"type":"text","text":"Why is the build failing on main?"}]}' --reason 'Ask about the failing build'
```

## sessions messages ratings

List your good and bad ratings of the messages in a session.

```bash
everruns sessions messages ratings [OPTIONS] [SESSION]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# See which replies you rated in a session
everruns sessions messages ratings session_01h9
```

## sessions messages list

List materialized messages in a session, optionally limited to the most recent N.

```bash
everruns sessions messages list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |
| `--limit <LIMIT>` | Maximum number of items returned in this page. |

Example:

```bash
# Read the latest messages of a session
everruns sessions messages list session_01h9 --limit 20
```

## sessions messages rate

Rate one message in a session good or bad, with an optional comment. A null rating clears your rating.

```bash
everruns sessions messages rate [OPTIONS] [SESSION] [MESSAGE_ID]
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Session's prefixed public identifier. |
| `--message-id <MESSAGE_ID>` | Message to rate (`msg_...`), a user or agent message of the session. |
| `--comment <COMMENT>` | Optional note on what was good or bad. |
| `--rating <RATING>` |  |

Example:

```bash
# Mark an agent reply as a bad answer
everruns sessions messages rate session_01h9 msg_01h9 --rating bad --comment 'Ignored the failing test' --reason 'Flag a wrong answer'
```

## sessions participants add

Add a member participant to a session.

```bash
everruns sessions participants add [OPTIONS] --kind <kind> --session <session_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Agent to add when `kind` is `agent`. |
| `--kind <KIND>` | Required. Kind of actor participating in a session. One of `agent`, `user`. |
| `--role <ROLE>` |  |
| `--session <SESSION_ID>` | Required. Session that receives the participant. |

Example:

```bash
# Bring a user into a running session
everruns sessions participants add --session session_01h9 --kind user --reason 'Bring in the on-call reviewer'
```

## sessions participants leave

Mark a session member participant as having left.

```bash
everruns sessions participants leave [OPTIONS] --participant-id <participant_id> --session <session_id>
```

| Flag | Description |
|---|---|
| `--participant-id <PARTICIPANT_ID>` | Required. Participant row to mark as left. |
| `--session <SESSION_ID>` | Required. Session that owns the participant. |

Example:

```bash
# Remove one participant from a session
everruns sessions participants leave --session session_01h9 --participant-id part_01h9 --reason 'Review finished'
```

## sessions participants list

List the participant history for a session.

```bash
everruns sessions participants list [OPTIONS] --session <session_id>
```

| Flag | Description |
|---|---|
| `--session <SESSION_ID>` | Required. Session whose participant history should be returned. |

Example:

```bash
# See who is attached to a session
everruns sessions participants list --session session_01h9
```

## sessions platform-chat ensure

Open the current user's permanent platform conversation.

```bash
everruns sessions platform-chat ensure [OPTIONS]
```

Example:

```bash
# Open your permanent platform conversation, creating it on first use
everruns sessions platform-chat ensure --reason 'Open the platform chat'
```

## sessions resources list

List all resources registered in a session.

```bash
everruns sessions resources list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# List the files and other resources registered in a session
everruns sessions resources list session_01h9
```

## sessions sandbox get

Inspect a Session's primary Sandbox and what it may touch.

```bash
everruns sessions sandbox get [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# See what a session's sandbox can reach
everruns sessions sandbox get session_01h9
```

## sessions sandbox manage

Pause, resume, or delete the managed sandbox for a session.

```bash
everruns sessions sandbox manage [OPTIONS] --action <action> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--action <ACTION>` | Required. Operator action to take against a session's managed sandbox. One of `pause`, `resume`, `delete`. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Pause a session's sandbox to stop paying for idle compute
everruns sessions sandbox manage --session-id session_01h9 --action pause --reason 'Idle overnight'
```

## sessions sse stream

Stream events via SSE. Not supported in bash mode.

```bash
everruns sessions sse stream [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Watch a session's events live as they happen
everruns sessions sse stream session_01h9
```

## sessions storage keys list

List all key-value pairs stored for a session.

```bash
everruns sessions storage keys list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Inspect the key-value pairs a session has stored
everruns sessions storage keys list session_01h9
```

## sessions storage secrets batch

Encrypt and store multiple session secrets in one request.

```bash
everruns sessions storage secrets batch [OPTIONS] --secrets <secrets> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--secrets <SECRETS>` | Required. Secret values keyed by name, e.g. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |

Example:

```bash
# Give a session several credentials in one call
everruns sessions storage secrets batch --session-id session_01h9 --secrets '{"SERVICE_TOKEN":"s3cr3t","DB_PASSWORD":"hunter2"}' --reason 'Provision credentials for the deploy run'
```

## sessions storage secrets delete

Delete a user-managed encrypted session secret by name.

```bash
everruns sessions storage secrets delete [OPTIONS] --name <name> --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Required. Exact secret name to delete. |
| `--session-id <SESSION_ID>` | Required. Session that owns the secret. |

Example:

```bash
# Revoke a credential from a session
everruns sessions storage secrets delete --session-id session_01h9 --name SERVICE_TOKEN --reason 'Token rotated'
```

## sessions storage secrets list

List all secrets stored for a session without revealing values.

```bash
everruns sessions storage secrets list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# See which secrets a session has, without their values
everruns sessions storage secrets list session_01h9
```

## sessions tasks cancel

Request cooperative cancellation of a session task.

```bash
everruns sessions tasks cancel [OPTIONS] --session-id <session_id> --task-id <task_id>
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--task-id <TASK_ID>` | Required. Task's prefixed public identifier. |

Example:

```bash
# Ask a runaway background task to stop
everruns sessions tasks cancel --session-id session_01h9 --task-id task_01h9 --reason 'Stuck on a dead host'
```

## sessions tasks get

Get one session task with its recent message thread.

```bash
everruns sessions tasks get [OPTIONS] --session-id <session_id> --task-id <task_id>
```

| Flag | Description |
|---|---|
| `--after-id <AFTER_ID>` | Return only messages newer than this message ID (exclusive cursor). When omitted, the most re... |
| `--limit <LIMIT>` | Maximum number of messages to return. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--task-id <TASK_ID>` | Required. Task's prefixed public identifier. |

Example:

```bash
# Read a task's state and its recent messages
everruns sessions tasks get --session-id session_01h9 --task-id task_01h9
```

## sessions tasks list

List background tasks owned by a session.

```bash
everruns sessions tasks list [OPTIONS] [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |
| `--kind <KIND>` | Optional kind filter (subagent, external_agent, background_tool, ...). |
| `--state <STATE>` | Optional state filter (queued, running, awaiting_input, succeeded, failed, canceled). |

Example:

```bash
# See what background tasks a session is running
everruns sessions tasks list session_01h9 --state running
```

## sessions tasks messages post

Send an inbound message to a session task.

```bash
everruns sessions tasks messages post [OPTIONS] --session-id <session_id> --task-id <task_id>
```

| Flag | Description |
|---|---|
| `--content <CONTENT>` | Structured message parts (alternative to `text`). |
| `--in-reply-to <IN_REPLY_TO>` | Input request ID this message answers, when applicable. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--task-id <TASK_ID>` | Required. Task's prefixed public identifier. |
| `--text <TEXT>` | Plain-text message (alternative to `content`). |

Example:

```bash
# Answer a task that is waiting for input
everruns sessions tasks messages post --session-id session_01h9 --task-id task_01h9 --text 'Use the staging database' --reason 'Unblock the migration task'
```

## sessions tasks push-configs create

Create a per-task push-notification config.

```bash
everruns sessions tasks push-configs create [OPTIONS] --session-id <session_id> --task-id <task_id> --url <url>
```

| Flag | Description |
|---|---|
| `--event-filter <EVENT_FILTER>` | Events that trigger delivery. Repeatable. |
| `--secret <SECRET>` | Optional HMAC-SHA256 signing secret. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--task-id <TASK_ID>` | Required. Task's prefixed public identifier. |
| `--url <URL>` | Required. URL to POST task events to. |

Example:

```bash
# Get a webhook call when a task finishes
everruns sessions tasks push-configs create --session-id session_01h9 --task-id task_01h9 --url https://example.com/hooks/tasks --secret "$WEBHOOK_SECRET" --reason 'Notify the deploy pipeline'
```

## sessions tasks push-configs delete

Delete a per-task push-notification config.

```bash
everruns sessions tasks push-configs delete [OPTIONS] --config-id <config_id> --session-id <session_id> --task-id <task_id>
```

| Flag | Description |
|---|---|
| `--config-id <CONFIG_ID>` | Required. Push config public id (tpc_...). |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--task-id <TASK_ID>` | Required. Task's prefixed public identifier. |

Example:

```bash
# Stop webhook deliveries for a task
everruns sessions tasks push-configs delete --session-id session_01h9 --task-id task_01h9 --config-id cfg_01h9 --reason 'Pipeline retired'
```

## sessions tasks push-configs list

List per-task push-notification configs.

```bash
everruns sessions tasks push-configs list [OPTIONS] --session-id <session_id> --task-id <task_id>
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier. |
| `--task-id <TASK_ID>` | Required. Task's prefixed public identifier. |

Example:

```bash
# Check where a task's webhooks are delivered
everruns sessions tasks push-configs list --session-id session_01h9 --task-id task_01h9
```

## sessions tool-results submit

Submit client-side tool results back to a waiting session.

```bash
everruns sessions tool-results submit [OPTIONS] --tool-results <tool_results> [SESSION_ID]
```

| Flag | Description |
|---|---|
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |
| `--tool-results <TOOL_RESULTS>` | Required. Results for the pending client-side tool calls, one entry per `tool_call_id`. |

Example:

```bash
# Return the result of a client-side tool call to a waiting session
everruns sessions tool-results submit session_01h9 --tool-results '[{"tool_call_id":"toolu_01","result":{"url":"https://example.com/orders/42"}}]' --reason 'Deliver the tool output'
```

## sessions trace get

Summarize a session's trace: turn, step and error totals, a minimap of turn buckets, and the turns that failed.

```bash
everruns sessions trace get [OPTIONS] --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--buckets <BUCKETS>` | Minimap buckets wanted, 1 to 500. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier (a path parameter). |

Example:

```bash
# Count a session's turns, steps and errors
everruns sessions trace get --session-id session_01h9
```

## sessions trace turns list

List a page of a session's turns with their steps: model calls, tool calls, approvals, sub-agents and messages, with repeated calls folded into batches.

```bash
everruns sessions trace turns list [OPTIONS] --session-id <session_id>
```

| Flag | Description |
|---|---|
| `--after <AFTER>` | Turns after this turn number. |
| `--around <AROUND>` | Turns centered on this turn number. |
| `--before <BEFORE>` | Turns before this turn number. |
| `--limit <LIMIT>` | Turns per page, 1 to 50. |
| `--sequence <SEQUENCE>` | Turns centered on the turn holding this event sequence. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier (a path parameter). |

Example:

```bash
# Open the latest turns of a long session
everruns sessions trace turns list --session-id session_01h9 --limit 20
```

## sessions trace turns events list

List the raw events of one turn of a session's trace, without deltas; small payloads are inlined.

```bash
everruns sessions trace turns events list [OPTIONS] --session-id <session_id> --turn <turn>
```

| Flag | Description |
|---|---|
| `--after-sequence <AFTER_SEQUENCE>` | Only events after this sequence. |
| `--limit <LIMIT>` | Events per page, 1 to 500. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier (a path parameter). |
| `--turn <TURN>` | Required. Turn number (a path parameter). |

Example:

```bash
# Read the raw events of one turn
everruns sessions trace turns events list --session-id session_01h9 --turn 12
```

## sessions trace turns steps get

Read one step of a session's trace in full: tool input and output, the model request and response, and its raw events.

```bash
everruns sessions trace turns steps get [OPTIONS] --session-id <session_id> --step <step> --turn <turn>
```

| Flag | Description |
|---|---|
| `--full` | Return payloads whole, however large. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier (a path parameter). |
| `--step <STEP>` | Required. Step number within the turn (a path parameter). |
| `--turn <TURN>` | Required. Turn number (a path parameter). |

Example:

```bash
# Read a tool call's full input and output
everruns sessions trace turns steps get --session-id session_01h9 --turn 12 --step 4 --full true
```

## sessions trace turns steps list

Page through the steps of one turn of a session's trace, optionally only the failed ones.

```bash
everruns sessions trace turns steps list [OPTIONS] --session-id <session_id> --turn <turn>
```

| Flag | Description |
|---|---|
| `--errors-only` | Only failed steps. |
| `--from-step <FROM_STEP>` | First step of the range, inclusive. |
| `--limit <LIMIT>` | Steps per page, 1 to 500. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier (a path parameter). |
| `--to-step <TO_STEP>` | Last step of the range, inclusive. |
| `--turn <TURN>` | Required. Turn number (a path parameter). |

Example:

```bash
# List only the failed steps of turn 12
everruns sessions trace turns steps list --session-id session_01h9 --turn 12 --errors-only true
```

## sessions trace turns steps request list

Page through the messages a model call in a session's trace was sent, with each message's full content.

```bash
everruns sessions trace turns steps request list [OPTIONS] --session-id <session_id> --step <step> --turn <turn>
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Messages per page, 1 to 200. |
| `--offset <OFFSET>` | First message index. |
| `--role <ROLE>` | Only messages of this role: `system`, `user`, `assistant` or `tool`. |
| `--session-id <SESSION_ID>` | Required. Session's prefixed public identifier (a path parameter). |
| `--step <STEP>` | Required. Step number of a model call (a path parameter). |
| `--turn <TURN>` | Required. Turn number (a path parameter). |

Example:

```bash
# See exactly what a model call was sent
everruns sessions trace turns steps request list --session-id session_01h9 --turn 12 --step 3
```
