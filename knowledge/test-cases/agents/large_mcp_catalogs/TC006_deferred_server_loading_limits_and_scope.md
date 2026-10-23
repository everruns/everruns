---
type: Test Case
title: "TC006: Large MCP Catalogs - Deferred Loading Limits and Scope"
description: "Verify CLI search loads at most eight matching deferred servers per query, preserves pending servers, respects aggregate bounds, and cannot cross turn scope."
tags:
  - everruns
  - test-case
  - agents
  - mcp
  - tool-search
---
# TC006: Large MCP Catalogs - Deferred Loading Limits and Scope

## Description

Verify CLI search loads at most eight matching deferred servers per query,
preserves pending servers, respects aggregate bounds, and cannot cross turn
scope.

## Preconditions

- A controlled MCP fixture with ten visible deferred servers whose names or
  descriptions match one query, plus one matching server placeholder excluded
  from the current turn's visible-tool allowlist.
- Each visible server exposes a harmless, uniquely named read operation and a
  deterministic result. Record every listing and call in the fixture audit.
- A host test harness can inspect `tools --help`, run multiple `tools` commands
  in one shell call, and apply the current visible-tool allowlist.
- A separate disposable session can be seeded near the 10,000-tool or 32 MiB
  shell catalog ceiling to exercise atomic load rejection.

## Test Data

| Catalog state | Query/result | Expected observable behavior |
|---|---|---|
| Ten matching deferred servers in scope | `tools search "audit ledger"` | At most eight server listings; loaded tools appear in help and operation results; remaining placeholders stay `(not loaded)` |
| Two remaining deferred servers | Repeat the same search | The remaining servers can load and their tools become searchable |
| Matching server outside current scope | Search, source help, and direct call attempt | It is absent from help/search and cannot be loaded or invoked |
| Deferred server would exceed aggregate shell catalog bound | Search for its operation | No child operation from that server is added; its placeholder remains pending |

## Steps

1. Start the shell with all ten visible placeholders and the one out-of-scope
   placeholder. Capture initial `tools --help` output and current turn scope.
2. Run the same matching `tools search` query. Record which servers the fixture
   listed, the resulting operation matches, and the updated help output.
3. Repeat the search in the same shell call. Verify the remaining matching
   servers load and can be invoked with their authoritative schemas.
4. Search for the out-of-scope server, request `tools <server> --help`, and
   attempt its operation by name. Capture CLI output and fixture audit.
5. In the near-limit session, search for the deferred server that would exceed
   the aggregate limit. Inspect help, search output, and fixture audit for
   partial additions or calls.

## Expected Result

- A single search loads no more than eight matching deferred servers. Untouched
  servers remain `(not loaded)` and can be loaded by a later search; a server's
  returned tools enter the shell catalog atomically.
- Search, help, loading, and invocation remain limited to the current turn's
  visible-tool allowlist. A loaded child tool retains the scope of its parent
  placeholder, and scope rejection never produces a fixture call.
- A load that would exceed 10,000 tools or 32 MiB adds none of that server's
  operations. The server remains pending, with no partial operation available
  for search, help, or invocation.

## Source Contract

[Tools in Shell](../../../execution/tools-in-shell.md), [Native and CLI Tool
Surfaces](../../../execution/tool-surfaces.md), and [Large MCP
Catalogs](../../../execution/large-mcp-catalogs.md).
