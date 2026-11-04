---
type: Test Case
title: "TC004: Large MCP Catalogs - Dynamic Changes and Replay"
description: "Verify dynamic server and plugin changes, schema redefinitions, authorization revocation, restart, and compaction preserve current execution authority."
tags:
  - everruns
  - test-case
  - agents
  - mcp
  - tool-search
---
# TC004: Large MCP Catalogs - Dynamic Changes and Replay

## Description

Verify dynamic server and plugin changes, schema redefinitions, authorization revocation, restart, and compaction preserve current execution authority.

## Preconditions

- Canonical DB-backed stack with a real LLM, one disposable agent/session, a controlled MCP fixture, and a plugin or capability fixture that can be attached, removed, and redefined.
- Use a warm session and record agent/session/actor IDs, fixture revisions and audits, authorization state, model/provider, starting event cursor, initial prompt prefix, and schema digest for each operation.
- Operations are harmless reads or mutations confined to disposable fixture state.

## Test Data

| Change | Initial state | Updated state |
| --- | --- | --- |
| Server attachment | Server absent | Authorized and attached with `read_item` |
| Plugin lifecycle | Plugin absent | Attached, redefined, then removed |
| MCP schema | `read_item` revision A | Same identity with changed required field and revision B |
| Authorization | Connection valid | Connection revoked after schema disclosure |
| Replay | Warm transcript | Worker restart and transcript compaction/replay |

## Steps

1. Search and invoke the initial server operation; record schema, prompt/tool baseline, and fixture audit.
2. Attach a second MCP server and the plugin. On the next turn, search for each new operation and invoke it. Redefine one plugin operation and the MCP schema, refresh discovery, then repeat with the new schema.
3. Remove the plugin and MCP attachment, then ask for each removed operation again. Verify removal from search and denial of direct/CLI invocation attempts.
4. Reattach the server, disclose its schema, revoke its credential or authorization, then attempt the same call over native and CLI routes.
5. Restart the worker and compact/replay the conversation. Repeat lookup and invocation for current and removed operations.
6. Compare no-change turns, additive changes, redefinitions, and removals using the serialized tool/prompt prefix and provider cache metadata where available.

## Expected Result

- Attachments become searchable on the next eligible lookup; removals and redefinitions update the catalog used for subsequent calls.
- A redefined operation uses the current authoritative schema and rejects old malformed arguments. No stale executor or schema is revived by a warm cache, worker restart, or compaction.
- Revoked authorization blocks execution on every surface even when the model transcript still contains the old operation name or schema.
- Tool disclosure changes follow the existing tool-change/replay behavior. Record whether the actual endpoint preserves its prompt prefix and reports cache use; do not claim all provider transports have identical cache behavior.

## Source Contract

[Large MCP Catalogs](../../../execution/large-mcp-catalogs.md), [Native and CLI Tool Surfaces](../../../execution/tool-surfaces.md), [cache-stable tool changes](../../../execution/cache-stable-tool-changes.md), and [MCP recovery](../mcp_recovery/TC001_last_listed_tools.md).
