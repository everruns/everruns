# TC002: Discover and Execute via MCP Tier 2 Tools

## Description

Verify that an external client can use Everruns' MCP Tier 2 `discover` and `execute` tools together: first search the catalog to find the right operations, then execute those operations to inspect harnesses, create an agent, and fetch it back.

This test exercises the `/mcp` endpoint's catalog search path plus the bashkit-backed execution path where discovered operations become built-in shell commands. It also validates the positional-ID rewrite for `get_agent <id>`.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- MCP endpoint reachable at `/mcp`
- Caller authenticated for `/mcp` (DEV_MODE, session cookie, API key, or MCP OAuth token)
- Default org has built-in harnesses provisioned (`base`, `generic`, `platform-chat`)

## Test Data

| Field | Value |
|-------|-------|
| Discover query A | `agent` |
| Discover query B | `harnesses` |
| Agent name | `catalog-smoke-bot` |
| Agent display name | `Catalog Smoke Bot` |
| Agent system prompt | `Reply with the single word catalog.` |

## Steps

### Happy path — discover then execute

1. **Call MCP `discover`** with query `agent`.

2. **Verify the response** includes `create_agent` and enough surrounding context to show that catalog search is matching operation names/descriptions rather than returning an empty result.

3. **Call MCP `discover`** with query `harnesses`.

4. **Verify the response** includes `list_harnesses`. This proves the client can find related operations without prior knowledge of exact command names.

5. **Call MCP `execute`** with:

   ```bash
   list_harnesses
   ```

6. **Verify the output** shows:
   - A JSON payload
   - Seed harnesses including `generic`

7. **Call MCP `execute`** with:

   ```bash
   create_agent --name 'catalog-smoke-bot' --display_name 'Catalog Smoke Bot' --system_prompt 'Reply with the single word catalog.'
   ```

8. **Verify the output** returns a created agent with:
   - An `agent_...` ID
   - `name: "catalog-smoke-bot"`
   - `display_name: "Catalog Smoke Bot"`

9. **Call MCP `execute`** again using the positional form:

   ```bash
   get_agent <agent_id_from_step_7>
   ```

10. **Verify the positional form succeeds** even without an explicit `--id` flag.

### Negative path A — missing discover query

11. **Call MCP `discover`** with no `query` and no `all: true`.

12. **Expected:** MCP returns a tool error explaining that a query is required (or that `all: true` must be set). No transport error, no 500, no empty success payload.

### Negative path B — bad execute builtin

13. **Call MCP `execute`** with:

   ```bash
   definitely_not_a_real_command
   ```

14. **Expected:** MCP returns a command-not-found/tool-error style response. The server stays healthy and subsequent valid `discover`/`execute` calls still work.

## Expected Result

### Discover

- `discover` returns catalog matches for natural-language queries.
- Query `agent` finds `create_agent`.
- Query `harnesses` finds `list_harnesses`.
- Missing-query input returns a clear validation error.

### Execute

- `execute` runs catalog operations as built-in commands in the Tier 2 execution environment.
- `list_harnesses` returns seeded harnesses, including `generic`.
- `create_agent` succeeds and returns JSON with an `agent_...` ID.
- `get_agent <id>` returns the created agent.
- Positional `get_agent <id>` also succeeds.
- Invalid builtins fail gracefully without breaking the MCP endpoint.

## Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Discover returns no matches for obvious queries | Catalog indexing/search wiring broken |
| `execute` cannot see discovered commands | Tier 2 registration mismatch between `discover` and `execute` |
| `get_agent <id>` fails | Positional-argument rewrite regression in MCP `execute` |
| `list_harnesses` missing `generic` | Seed harness provisioning or org initialization issue |
| Bad builtin crashes transport | MCP endpoint error handling regression |

## Notes

- This is intentionally an MCP-client workflow, not a browser-chat flow. It covers the Tier 2 surface described in `knowledge/execution/apis.md` and `knowledge/integrations/mcp.md`.
- Exact `discover` wording may change. Match on the presence of relevant operation names and a non-error response rather than exact prose.
