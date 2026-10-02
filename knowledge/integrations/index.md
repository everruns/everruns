# MCP, integrations, and apps

* [MCP (Model Context Protocol) Specification](mcp.md) - MCP server endpoint, OAuth 2.1 authentication, protocol, security.
* [MCP Server Specification](mcp-servers.md) - MCP client remote server registration, CRUD API, tool naming, execution.
* [Runtime MCP Client Specification](runtime-mcp.md) - MCP client in the in-process runtime: shared `everruns-mcp` crate, transport abstraction (HTTP + optional stdio), pluggable auth.
* [Agent MCP Attachments (acts-as semantics)](agent-mcp-attachments.md) - Make who an MCP server acts as an explicit, fail-closed property of an Agent attachment; org MCP servers become presets; one MCP surface per Agent.
* [MCP Events: session webhooks out, agent triggers in](mcp-events.md) - /mcp clients subscribe to session webhooks; agents subscribe to their MCP servers' events as `mcp_event` triggers.
* [Inbound Form Mode Elicitation](mcp-form-elicitation.md) - Answering an attached MCP server's form mode elicitation through ask_user, and the trust rules that shape it.
* [Integrations](integrations.md) - Integration specs index.
* [Apps](apps.md) - Frozen App compatibility data and permanent ingress aliases.
* [Agent Exposure (retiring the App abstraction)](agent-exposure.md) - Make Agent the addressable entity by re-homing channels as Endpoints and folding invocation into Triggers, retiring App.
* [Public Chat (Hosted Chat App)](public-chat.md) - Public Chat (hosted, isolated chat app), product spec/proposal.
* [Legacy App Invocation Aliases](app-invocation-channels.md) - Frozen App-shaped aliases for endpoint-owned webhook and schedule ingress.
* [Endpoint Authentication](endpoint-auth.md) - Shared inbound auth framework for Agent-owned endpoints.
* [Legacy App API Keys](app-api-keys.md) - Frozen execution-only credentials for endpoint-owned native session ingress.
* [AG-UI Channel](ag-ui.md) - AG-UI 1.0 channel: wire types, runtime-event projection, the consumer pipeline, and the 1.0 rules each side keeps.
* [A2A Channel](a2a-channel.md) - A2A inbound channel.
* [A2A Capability](a2a-capability.md) - A2A outbound delegation capability.
* [AG-UI Capability](ag-ui-capability.md) - AG-UI outbound delegation: configured external AG-UI agents as spawn_agent targets backed by session tasks.
* [FCP (Free Communication Protocol) channel](fcp-channel.md) - FCP inbound channel.
* [Messaging Integrations](messaging-integrations.md) - Messaging integrations.
* [Slack Integration Modernization](slack-modernization.md) - Gap analysis of the Slack channel against the current Slack agent platform, with a prioritized set of improvements.
* [Slack Agent Actions](slack-agent-actions.md) - Why Slack approvals, task progress, and the second-identity problem are one missing capability.
* [Slack One-Click Install](slack-one-click-install.md) - What a live PoC established about creating per-agent Slack apps programmatically.
* [Per-agent GitHub Apps](github-apps.md) - One-click GitHub App per agent identity: one installation for GitHub tools, MCP and events.
* [GitHub review and security agent templates](github-agent-templates.md) - PR Reviewer and Security Scanner: guided agent examples, deterministic repeat suppression, settings the tools enforce.
* [Plugins](plugins.md) - Plugin host: marketplaces and cross-host plugin packages installed as capabilities.
* [Model Router Specification](model-router.md) - Model Routers.
