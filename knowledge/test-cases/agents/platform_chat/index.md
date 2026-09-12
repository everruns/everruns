# Platform chat (Agent workflow)

* [Evidence](evidence/) - Evidence captured while running these cases.

* [TC001: Create and Run an Agent via Platform Chat](TC001_create_and_run_agent_via_chat.md) - Verify that a user can use the Platform Chat global session to create a new agent end-to-end and then ask Platform Chat to invoke that agent — all from a single conversation.
* [TC002: Discover and Execute via MCP Tier 2 Tools](TC002_discover_and_execute_via_mcp.md) - Verify that an external client can use Everruns' MCP Tier 2 `discover` and `execute` tools together: first search the catalog to find the right operations, then execute those operations to inspect harnesses, create an...
* [TC003: Answer Platform Questions from Embedded Docs](TC003_answer_from_embedded_platform_docs.md) - Verify that Platform Chat answers a repo-specific product question by consulting the embedded platform docs mounted at `/workspace/docs`, not by relying only on generic model knowledge.
* [TC004: Create a Scheduled Agent via Platform Chat](TC004_create_scheduled_agent_via_platform.md) - Verify that Platform Chat can discover models and control-plane operations, create an Agent with an explicit default model, and create an Agent Trigger for recurring autonomous work.
* [TC005: Ground Plugin and Connection State Before Agent Creation](TC005_ground_plugin_agent_preflight.md) - Verify that Platform Chat distinguishes operation discovery from resource inspection and grounds an Agent-creation confirmation in authoritative plugin, capability, Agent, connector, and current-user connection reads.
