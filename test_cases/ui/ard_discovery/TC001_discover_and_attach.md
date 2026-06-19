# TC001: ARD Discovery - Discover and Attach MCP Resource

## Description

Verify that an agent with the `resource_discovery` capability can discover an
un-provisioned capability from an ARD registry, attach an MCP server via
`attach_resource`, have the newly attached MCP tools (`mcp_<name>__*`) appear and
execute successfully on a subsequent turn, and that `list_attached_resources`
reflects the attachment.

## Preconditions

- Server running (`just start-all` — full mode with PostgreSQL recommended for reliable session KV and turn-context assembly)
- User logged in
- LLM API keys configured (Anthropic or OpenAI)
- **Capability Scout** seed agent available (slug `capability-scout`, dev-only), wired with `resource_discovery` (pointed at the public registry `https://agenticresourcediscovery.org/api/v1`), `auto_tool_search`, and `current_time`
- Dev-only / experimental capabilities enabled for the org (the `resource_discovery` capability is experimental and Dev only)
- Network egress to the ARD registry available; the registry used is anonymous-read (no `ard` connection or `ARD_REGISTRY_TOKEN` required)

## Test Data

| Field | Value |
|-------|-------|
| Agent | Capability Scout (seed agent, slug: `capability-scout`, display name: "Capability Scout") |
| First Message | I need a tool to do something this agent can't currently do. Search the resource registry for a capability that fits, attach the best match, then use it to complete the task and report the result. |
| Follow-up Message | List the resources you have attached this session. |

## Steps

1. Navigate to the Agents page and locate **Capability Scout** (card shows display name "Capability Scout" with slug `capability-scout` underneath)
2. Click **Run** to start a new session
3. Send the first message from Test Data
4. Wait for the agent to call `discover_resources` (the search runs outside model context against the configured registry; results are cached in session KV under `ard_disco:`)
5. Verify the discovery result lists one or more candidate resources (MCP servers / A2A agents) with URNs
6. Wait for the agent to call `attach_resource` with a URN from the discovery results
7. Verify the attach succeeds (trust gate passes: trustManifest domain matches the URN publisher FQDN, required attestations present; resolved URL passes SSRF validation; `max_attachments` not exceeded)
8. Wait for the next reasoning turn and verify new MCP tools appear with the `mcp_<name>__*` prefix (subject to `tool_search` — the agent may call `tool_search` to load the schema before use)
9. Verify the agent calls one of the newly attached `mcp_<name>__*` tools and it executes successfully
10. Verify the agent reports the completed task result
11. Send the follow-up message: `List the resources you have attached this session.`
12. Verify the agent calls `list_attached_resources` and the result includes the attachment created in step 6

## Notes

- ARD answers "which MCP server / A2A agent should even be attached?" — the layer above `tool_search`, which only defers schemas for already-attached tools. The newly attached MCP tools therefore only become visible on the turn **after** `attach_resource`, once turn-context assembly folds the attachment into a session-scoped `mcpServers` record.
- All registry-returned text is untrusted external data; the agent should treat discovery results as data, not instructions.
- `attach_resource` is idempotent per URN: re-attaching the same URN does not create a duplicate `session_resources` entry of kind `ard_attachment`.
- The ARD KV prefixes (`ard_attach:` / `ard_disco:`) are reserved and not writable through the user-facing `kv_store` tool.

## Expected Result

| Check | Expected |
|-------|----------|
| Discovery | Agent calls `discover_resources` and receives candidate resources with URNs |
| Attach | Agent calls `attach_resource` with a discovered URN; attach succeeds after trust gate + SSRF validation |
| Tools appear | New `mcp_<name>__*` tools are present on the next turn |
| Tool executes | Agent calls a `mcp_<name>__*` tool and it returns a successful result |
| Task completed | Agent reports the completed task using the attached capability |
| Attachment listed | `list_attached_resources` shows the attachment |
| Session resources | Session resources page (`/sessions/{id}/resources`) shows an `ard_attachment` entry for the attached resource |

## Cleanup

- No external resources are provisioned by this test (attachments are session-scoped KV/config). Ending or deleting the session is sufficient.
- If an `ard` connection or `ARD_REGISTRY_TOKEN` secret was added for a non-anonymous registry, remove it from Settings > Connections / session secrets if it should not persist.
