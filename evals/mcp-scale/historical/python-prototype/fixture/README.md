# MCP scale fixture

A deterministic fake operations platform exposed as an MCP server. It uses only
the Python standard library and supports newline JSON-RPC over stdio or the
2025-06-18 Streamable HTTP request/response path.

```bash
python3 evals/mcp-scale/fixture/server.py \
  --transport http --host 127.0.0.1 --port 8787 \
  --tools 600 --page-size 100 \
  --state-file /tmp/mcp-scale-state.json \
  --audit /tmp/mcp-scale-calls.jsonl
```

For a local stdio client, run `python3 evals/mcp-scale/fixture/server.py
--transport stdio --tools 600 --page-size 100`. The catalog can also be loaded
by an evaluator with `from server import Fixture, build_catalog`; the keyword
forms `tool_count=` and `tools_count=` are both accepted. Every catalog entry
has `name`, `description`, `inputSchema`, `annotations`, and `_meta` fields.

The catalog has 19 real operational tools across deployments, observability,
incidents, tickets, service catalog, runbooks, and change management. Remaining
entries are deterministic distractors, with generated names such as
`inspect_billing_record_001`. Tool search can identify the real actions from
their descriptions and schemas.

The baseline state includes failed `billing-api` staging deployment
`dep-bill-stg-2042` (version `3.8.1`, previous version `3.8.0`), healthy
production deployment `dep-bill-prd-9911`, incident `INC-427`, and ticket
`OPS-8421`. A call to `rollback_deployment` must name the exact deployment and
reason. It updates only the targeted deployment, persists to `--state-file`
when supplied, and leaves production unchanged. The read-only case can resolve
incident ownership through `get_incident` and follow `linked_ticket` to
`get_ticket`.

`tools/list` returns at most `--page-size` entries and follows the standard
`nextCursor` response field. Calls return both JSON text content and
`structuredContent`. Tool arguments are checked against each catalog schema,
including required fields, nested object properties, `additionalProperties`,
enums, and numeric bounds. `--audit` appends one JSON object per catalog list
or tool-call attempt.

Run protocol, pagination, state, and validation tests with:

```bash
python3 -m unittest discover -s evals/mcp-scale/fixture -p 'test_*.py'
```
