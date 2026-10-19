---
type: Test Case
title: "TC001: MCP - Tool Listing Failure and Recovery"
description: "Verify transient listing failure preserves the same session's last known tools while successful removal and identity changes cannot revive stale tools."
tags:
  - everruns
  - test-case
  - agents
  - mcp-recovery
---
# TC001: MCP - Tool Listing Failure and Recovery

## Description

Verify transient listing failure preserves the same session's last known tools while successful removal and identity changes cannot revive stale tools.

## Preconditions

- Canonical stack with real LLM, durable session storage, and a controlled MCP endpoint.
- Disposable agent with an explicitly attached server alias and harmless `read_marker` tool.
- Endpoint can fail tools/list while keeping tools/call available, then return a successful empty list. Do not use an uncontrolled public endpoint.
- Record exact session ID, actor identity, tool name/schema, and model-call tool definitions.

## Test Data

| Variant | Expected definitions |
| --- | --- |
| Successful first listing | Current tool |
| Transient tools/list failure | Same session's last listed tool |
| Successful empty listing | No removed tool |
| Failure after empty listing | No resurrected tool |
| New identity/session without discovery | No borrowed tools from another caller |

## Steps

1. Run a turn that discovers and invokes `read_marker`; inspect actual model definitions and tool result.
2. Fail tools/list, expire the controlled fixture's discovery cache, and run another turn in the same session. Keep tools/call available.
3. Return a successful empty listing, refresh discovery, and inspect the next turn's definitions.
4. Fail listing again and inspect another turn.
5. Remove or revoke the attachment, then inspect a new turn. Repeat first discovery under a separate caller/session whose listing has never succeeded.

## Expected Result

- Transient listing failure retains the prior schema for the same session and allows the harmless call when the upstream is available.
- A successful empty list replaces prior knowledge; later failure cannot resurrect removed tools.
- Revoked/removed attachments and another caller's identity do not inherit stale definitions or access.
- Actual upstream tools/call failure is surfaced as failure, not a fabricated successful result.

## Source Contract

[Per-turn definitions and recovery](../../../../crates/server/src/domains/mcp_servers/deferred/mod.rs) and [identity-scoped discovery cache](../../../../crates/server/src/domains/mcp_servers/scoped_mcp/mod.rs).
