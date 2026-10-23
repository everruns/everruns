---
type: Test Case
title: "TC005: Large MCP Catalogs - Discovery Limits and Incomplete Catalogs"
description: "Verify exact discovery limits succeed while over-limit catalogs and pagination failures are reported explicitly without publishing partial catalogs."
tags:
  - everruns
  - test-case
  - agents
  - mcp
  - tool-search
---
# TC005: Large MCP Catalogs - Discovery Limits and Incomplete Catalogs

## Description

Verify exact discovery limits succeed while over-limit catalogs and pagination failures are reported explicitly without publishing partial catalogs.

## Preconditions

- Canonical DB-backed stack with a real LLM and controlled HTTP and stdio MCP fixtures that can emit arbitrary page sizes, byte totals, cursors, duplicate names, delays, and failures on a selected page.
- Record fixture revision, transport, configured credentials, session and actor IDs, cache state, starting event cursor, and exact tool/schema byte counts for every variant.
- Use disposable catalogs and harmless operations. Where a prior valid catalog is needed, record its complete identities and digests before each failure variant.
- Verify aggregate scoped catalog limits and the 128 MiB in-memory cache budget using a Framework runtime host; the DB-backed platform's persistent discovery cache has a separate lifecycle.

## Test Data

| Variant | Catalog behavior | Expected discovery |
| --- | --- | --- |
| Exact per-server limit | 10,000 tools on one server | Complete if all other limits are met |
| Above per-server limit | 10,001 tools on one server | Explicit rejection/incomplete status |
| Exact aggregate session limit | 10,000 tools across authorized servers | Complete if all other limits are met |
| Above aggregate session limit | 10,001 tools across authorized servers | Fail closed with empty aggregate and explicit trace error |
| Exact data and pagination bounds | 16 MiB cumulative response bodies and 256 pages | Complete if serialized definitions and all other limits are also met |
| Above byte/page bound | 16 MiB + 1 byte or 257 pages | Explicit rejection/incomplete status |
| Aggregate session bytes | 32 MiB across server-prefixed definitions, then 32 MiB + 1 byte | Exact bound completes; overflow fails closed with no truncation |
| Cursor faults | Repeated cursor, empty cursor, or duplicate tool name | Explicit protocol/catalog error |
| Late failure | Fail or time out on a later page | No partial catalog presented as complete |

## Steps

1. Discover a catalog at each per-server and aggregate exact boundary using both HTTP and stdio where the fixture supports the same behavior. Verify every page was requested and the final operation is available.
2. Repeat with each over-limit and malformed-pagination variant. Capture the user-visible turn result, runtime diagnostics, page audit, and catalog/cache state.
3. Seed a valid catalog, then fail a later refresh page. Check whether the runtime preserves or clears prior discovery according to the current transient-failure and authentication rules; record the exact outcome and source revision.
4. For authentication failure, revoke the credential before refresh, then verify old tool definitions cannot authorize invocation.
5. Repeat a failing variant after restart and after cache expiry to ensure no cached partial catalog is treated as complete.

## Expected Result

- A catalog exactly at every declared bound can complete when its cursor chain and all other limits are valid.
- A catalog above any bound, with duplicate names or invalid cursor progress, is reported as incomplete/error. Everruns does not expose a partial result as a complete catalog or invoke a tool from an incomplete refresh.
- Any retained prior snapshot follows the documented transient-failure policy; authentication failure never leaves prior authority usable. Record current source-derived behavior rather than assuming all failures preserve or clear the snapshot.
- The Framework's scoped discovery cache remains within its 128 MiB weighted budget per runtime instance. Cache admission or bypass is observable in runtime diagnostics; no performance outcome is implied.

## Source Contract

[Large MCP Catalogs](../../../execution/large-mcp-catalogs.md), [MCP discovery implementation](../../../../crates/core/src/mcp/discovery.rs), and [MCP recovery](../mcp_recovery/TC001_last_listed_tools.md).
