---
title: Large MCP catalogs and tool surfaces
description: Everruns paginates MCP catalogs, searches authorized server and tool metadata, and reveals bounded schemas through native calls or the session shell.
sidebar:
  label: Large MCP catalogs
appliesTo: [framework, platform, cloud]
---

Everruns reads MCP catalogs page by page, indexes the tools available to the
current agent, and reveals schemas as they are needed. This lets an agent work
with catalogs containing thousands of operations without sending every
parameter schema to the model at the start of a turn.

The runtime catalog, model-visible schemas, and tool-result history are separate
budgets. A session can index 2,000 operations and use only a few: the initial
native surface contains the search entry point and required eager tools; a
search adds the selected operations to a bounded working set. Unselected
schemas stay in the runtime. A deferred MCP server is enumerated when selected,
then its operations can be searched without sending its complete catalog to the
model. Native disclosure defaults to retaining up to 32 optional schemas, not
reserving 32 slots that every search must fill.

## Complete catalog discovery

The MCP client follows every `tools/list` cursor and keeps the negotiated
connection and authorization for each page. A catalog is accepted only when
the server reports that there are no more pages. Repeated or empty cursors,
duplicate names within one server, and discovery that exceeds a fixed
implementation bound fail explicitly; these bounds are not deployment settings.
Everruns does not treat a partial page sequence as a
complete catalog.

Per-server discovery is bounded to 10,000 tools, 16 MiB each for cumulative
response bodies and serialized tool definitions, 256 pages, and 60 seconds
total. Response bytes include protocol framing. The HTTP transport also retains its existing
per-request timeout. The Framework's scoped discovery cache has a 128 MiB
weighted budget per runtime instance, so one large catalog cannot make that
cache grow without bound. These
limits are safety bounds, not a claim that every provider or model can use
10,000 schemas at once.

Custom Framework egress adapters must implement `EgressService::send_stream`.
HTTP discovery consumes bounded response streams even when the MCP reply is
ordinary JSON, so response limits apply before buffering the complete body.

The Framework's scoped MCP discovery also limits the combined session catalog
to 10,000 tools and 32 MiB of serialized, server-prefixed definitions. If it exceeds either
bound, discovery fails closed with an explicit trace error; it does not return
a truncated catalog. Discovery limits are fixed in the implementation, not
deployment settings.

## Search and schema disclosure

In bounded native mode (`tool_search` with `bounded: true`) and in the `tools`
CLI, search uses a shared authorized index over server names and descriptions,
tool names and descriptions, and parameter schema text. A server description
can help find generic operations such as `search`, `get`, or `list`. Results
retain the server and operation identity, so same-named tools on different
servers remain distinct. If only a server description matches, search returns
a server refinement rather than labeling its tools as operation matches. The
CLI marks these separately and points to `tools <server> --help`. Legacy native
`tool_search` keeps its existing matcher and does not use this server-aware
index.

When a query names an exact operation, search returns matching authorized
identities without padding the result with weaker description matches. An
unqualified operation shared by several servers can still return several
identities; include the server name to disambiguate it. Broader task queries
continue to use ranked retrieval.

Names, descriptions and schema text are scored independently, so a large
parameter schema does not bury a strong name or description match. Complete
operation names accept spaces, hyphens or underscores: `update agent` can find
a registered `update_agent` operation. Current authorization still determines
which identities can appear.

By default, `tool_search` keeps its existing behavior. To opt into bounded
schema disclosure, set `bounded` and the three limits on the capability:

```json
{
  "ref": "tool_search",
  "config": {
    "bounded": true,
    "max_active_tools": 32,
    "max_active_bytes": 65536,
    "max_result_bytes": 32768
  }
}
```

The first two limits bound optional disclosed schemas only. Eager,
client-side, and `never_defer` tools remain in addition to those limits, and
fallback drivers may temporarily hold removals during a warm-prefix
transition. Held definitions must still fit the combined native tool budget:
eager definitions plus the configured optional count and byte limits. If a
lower live budget no longer admits the held list, the runtime releases those
removals immediately.
`max_result_bytes` bounds schemas returned by a search. Discovery remains
progressive: the model searches, receives schemas for a bounded set of
operations, then calls those operations using their registered schemas. A
search result does not create a second execution path. If matching schemas do
not fit in one result, search asks the model to narrow the query instead of
presenting omitted schemas as available. `omitted_tools` names byte-omitted
candidates within the same result budget; search one name exactly to request
that schema. These names do not make the operations callable. Removing an active schema does not
remove its earlier transcript history; this feature does not set a cumulative
history limit.

Tool search applies the current attachment and authorization rules before
returning matches, and the shell catalog is built from the current turn's
visible-tool scope. Search, help, and invocation all stay within that scope;
deferred child tools retain the scope of their server placeholder. The runtime
checks authority again when an operation is called. Removing a server or
revoking its connection therefore makes its operations unavailable even if a
prior turn disclosed their names or schemas.
When a server's schema changes, refreshed discovery supplies its current
definition; old arguments still go through current schema validation. A
transient refresh failure may leave the last successful catalog available
under the existing recovery policy. An authentication failure clears the
cached catalog, and an incomplete page sequence is never published as a
successful refresh.

## Native, CLI, and hybrid access

The same registered operation can be exposed as a native function, through the
session's `tools` command, or through both surfaces. Configure
[`tools_in_shell`](/capabilities/tools-in-shell/) with `mode: "cli"` for the
shell surface (the default), or `mode: "hybrid"` to retain native calls
alongside the CLI. Hybrid mode alone keeps native schemas eager; to progressively
reveal bounded native schemas, also enable bounded
[`tool_search`](/capabilities/tool-search/):

```json
{
  "capabilities": [
    { "ref": "tool_search", "config": { "bounded": true } },
    { "ref": "tools_in_shell", "config": { "mode": "hybrid" } }
  ]
}
```

With both capabilities configured, `tool_search` reveals selected native
schemas while the same catalog is searchable and callable through `tools`.
The CLI is useful when a script should chain operations or filter intermediate
results; native calls are useful for one typed operation. Both routes use the
same operation identity, schema validation, authorization, and execution
policy.

For CLI discovery, start with `tools search <task or server purpose>`, then
inspect one selected operation with `tools <server> <tool> --help`. Root and
server help are bounded to 64 entries and 12 KiB, with explicit omission counts
and search guidance. A selected operation's schema remains available through
its own help. Filtering intermediate results inside the shell also keeps large
response bodies out of the conversation.

When a CLI search matches deferred MCP servers, it loads at most eight per
query. Other matching servers stay marked as not loaded for a later search.
Each loaded server joins the shell catalog atomically; if it would exceed the
10,000-tool or 32 MiB catalog ceiling, none of that server's tools are added.

MCP servers and plugins join the runtime's operation registry when attached.
Attaching, removing, or redefining one updates what the next search or CLI
lookup can see. Existing authorization is checked during search and again
during execution, including after a warm cache, session restart, or history
compaction. Tool and prompt changes use the runtime's existing incremental
disclosure path; cache behavior can vary by provider and endpoint.

See [MCP](/features/mcp/) for server setup and
[Tool Search](/capabilities/tool-search/) for provider-specific search modes.

## Run the synthetic example

The repository's [Rust/Mira study](https://github.com/everruns/everruns/tree/main/evals/mcp-scale)
includes a working paginated MCP server and an agent using the Framework's
production discovery and execution paths. From an Everruns checkout, provide
`OPENROUTER_API_KEY` and run:

```bash
cd evals/mcp-scale
cargo run -- --offline-catalog
EVERRUNS_EVAL_TRIALS=1 EVERRUNS_EVAL_CONCURRENCY=1 \
  cargo run -- --run --sample readonly-ticket \
  --filter 'catalog_size=600,servers=1,' --runs-report /tmp/mcp-scale.jsonl
```

This runs the same read-only task through native, CLI, and hybrid surfaces
using Muse Spark 1.3 Contributor. The fixture has 600 tools, returns 100 tools
per page, and persists isolated synthetic state across MCP process launches.
The scorer requires completed operations and checks that no mutation occurred;
a model's final answer alone cannot pass. Change the catalog size to `2000`
to exercise a larger catalog. The study README documents the full matrix,
stress measurements, and limitations.
