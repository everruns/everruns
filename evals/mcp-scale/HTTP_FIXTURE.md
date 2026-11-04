# Loopback HTTP MCP fixture

The Rust `mcp_http_fixture` binary reuses the study's paginated catalog and
synthetic state dispatch. It binds only to a loopback address. Hosted MCP
requests must arrive through Everruns' tenant egress proxy using the configured
synthetic proxy secret; the fixture accepts only the absolute HTTP authorities
`example.com` and `93.184.215.14` on the `/mcp/{fixture_id}/{server_id}` path.
The address is a virtual public URL literal accepted only by this fixture; no
request is forwarded to that address. Using the literal avoids hostname DNS
resolution in per-request URL safety checks. The logical MCP URL to enter in the
UI can be:

```text
http://example.com/mcp/{fixture_id}/{server_id}
```

For timed multi-page cases, use the DNS-free fixture authority instead. Include
that exact authority if the local tenant egress configuration uses an allowlist:

```text
http://93.184.215.14/mcp/{fixture_id}/{server_id}
```

Root and MCP calls are controlled independently. The fixture process needs the
same `EVERRUNS_EGRESS_PROXY_SECRET` that the local tenant egress proxy uses.
Inject it through the existing local secret configuration; do not print it or
put it in a report. The hosted server's `EVERRUNS_EGRESS_PROXY_URL` should
point to the loopback proxy URL configured by the local DB stack.

## Start and check readiness

From the repository root:

```bash
source /workspace/.tools/activate.sh
CARGO_TARGET_DIR="$PWD/evals/mcp-scale/target" cargo build --locked --offline --manifest-path evals/mcp-scale/Cargo.toml --bin mcp_http_fixture
: "${EVERRUNS_EGRESS_PROXY_SECRET:?provide the local tenant proxy secret through the existing local secret configuration}"
CARGO_TARGET_DIR="$PWD/evals/mcp-scale/target" cargo run --locked --offline --manifest-path evals/mcp-scale/Cargo.toml --bin mcp_http_fixture -- --listen 127.0.0.1:8765
```

The process prints a readiness URL. The direct loopback health endpoint is
`GET http://127.0.0.1:8765/healthz`. Control endpoints are direct loopback only:

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` / `PUT` | `/control/{fixture_id}` | Read or merge-patch fixture-wide controls |
| `GET` / `PUT` | `/control/{fixture_id}/{server_id}` | Read or merge-patch a per-server override |
| `GET` | `/audit/{fixture_id}` | Ordered page and tool-call audit |

Use `curl --noproxy '*'` for these loopback control routes so local HTTP proxy
settings do not intercept them. The fixture audit records actual page cursors,
tool names, schema revision, call arguments and results, HTTP status, and page
response size. It never records proxy authorization credentials.
Generated catalogs are cached by fixture, server, and effective configuration
in a bounded 32-entry cache; control `PUT` requests invalidate affected entries.

## Control fields

Controls are JSON objects merged into the existing configuration. Defaults are
`scenario: "catalog"`, `catalog_size: 600`, `page_size: 100`,
`schema_revision: "A"`, `target_tool: "get_ticket"`, and `attached: true`.
`authorized_servers: null` allows all servers except `hidden` in the
`colliding-search` scenario. Useful fields are:

| Field | Effect |
| --- | --- |
| `scenario` | `catalog`, `colliding-search`, `dynamic`, or `deferred` |
| `catalog_size`, `page_size` | Catalog size and `tools/list` page size; catalog permits up to 20,000 tools for over-limit cases |
| `target_tool` | Final-page operation in catalog mode: `get_ticket` or `get_incident` |
| `schema_revision` | `A`/`B` version for the dynamic `read_item` schema and audit |
| `attached` | Detach an endpoint (list/call return 404) |
| `authorized_servers` | Array of allowed server IDs; omitted/null means all except the hidden colliding-search server |
| `deny` | Per-method HTTP status, e.g. `{"tools/list":401,"tools/call":403}`; only 401/403 are honored |
| `fault` | Pagination fault object with `kind`, zero-based `page`, and optional `delay_ms` |
| `catalog_json_bytes_target` | Pad the serialized full tools array to this byte size, when the target is at least its unpadded size |
| `padding_bytes` | Append this many bytes to the first tool description |

Pagination fault kinds are `repeated_cursor`, `empty_cursor`, `duplicate_name`,
`fail_page`, and `delay_page`. For example, this makes page index two fail:

```json
{"fault":{"kind":"fail_page","page":2}}
```

The target operation in catalog mode is deliberately placed last, after the
synthetic filler tools. With 100-tool pages, `get_ticket` or `get_incident` is
therefore discoverable only after traversing to the final page. Its schema
contains nested fields and its result identifies the selected server and
schema revision.

## Scenarios

- `catalog`: page-scale discovery, final-page operation, and deterministic
  ticket/incident reads. Configure `catalog_size` to 600 or 2,000 and
  `target_tool` to the operation under test.
- `colliding-search`: `alpha`, `beta`, and `hidden` expose the same `search`
  operation with different nested required fields (`filter.well_pad` and
  `filter.watershed_id`). Set the MCP server Description fields in the UI to
  `Geothermal drilling telemetry and field operations records` for alpha and
  `Urban water quality sampling and treatment records` for beta. The fixture
  cannot set host-level metadata itself. Hidden is omitted unless explicitly
  authorized.
- `dynamic`: exposes `read_item`; schema A requires `item.item_key`, and schema
  B also requires `item.revision_token` equal to `schema_revision`. Per-server
  control changes can model attachment or schema changes without restarting
  the fixture process.
- `deferred`: exposes a uniquely named nested-schema read per server. Use
  per-server overrides to create ten deferred endpoints and inspect which
  servers are listed on each search.

The control, health, and audit interface is not an agent tool. The model only
sees MCP operations exposed by the hosted server configuration. These controls
do not attach/remove Everruns plugins or authorize a server by themselves; use
the DB-backed UI/API to exercise those product behaviors.
