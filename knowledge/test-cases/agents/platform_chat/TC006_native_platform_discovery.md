---
type: Test Case
title: "TC006: Bounded Native Platform Discovery"
description: "Verify shared platform and MCP discovery, native structured calls, CLI composition, replay and current authorization."
tags: [everruns, test-case, agents, platform-chat]
---
# TC006: Bounded Native Platform Discovery

## Goal

Verify that Platform Chat discovers native platform operations and attached MCP
operations through one bounded registry, while retaining CLI composition.

## Preconditions

- Local stack on a distinct port prefix with a fresh test organization.
- Managed Platform Chat reconciled with native Platform, bounded Tool Search and
  hybrid Tools in Shell capabilities.
- A test MCP server with a generic read operation and distinctive public source
  description; a plugin capability with distinctive description text.

## Steps

1. Ask Chat to inspect the test organization's agents and models. Inspect the
   recorded model request: the initial optional catalog is absent; tool_search
   reveals a bounded set of platform functions with authoritative schemas.
2. Search using the capability's distinctive description. A source-only match
   identifies the capability for refinement rather than arbitrary operations.
   Add the desired operation term and verify the selected schema.
3. Repeat with the MCP source description and a generic operation term. Check
   that same-named platform and MCP operations retain separate canonical names.
4. Request a test-agent update after reading its manager context. Verify typed
   arrays and objects, reason, context revision, saved state and history.
5. Perform a bulk read using the shell. Search again after loading a deferred
   server in that same script; the existing capability context still matches.
6. Redefine a plugin schema, revoke the capability, and restart the worker.
   Replayed discovery uses current definitions; an old result cannot execute a
   revoked operation. The original provider baseline remains stable where the
   driver supports incremental disclosure.
7. Query a schema larger than the result budget. The result stays bounded,
   counts the omission and retains its canonical name when space permits.

## Expected result

Only authorized operations are searchable. Native and CLI calls retain the same
session organization, current caller, command policy and manager-context checks.
No catalog-wide prompt dump, hidden source disclosure, guessed schemas or stale
execution grants occur. Current inventory coverage is verified by the server's
native-catalog freshness test rather than a fixed historical command count.
