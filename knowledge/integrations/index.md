# MCP, integrations, and apps

* [Slack Response Policy](slack-response-policy.md) - Decide whether an agent should participate before starting a turn.

* [MCP (Model Context Protocol) Specification](mcp.md) - MCP server endpoint, OAuth 2.1 authentication, protocol, security.
* [MCP Server Specification](mcp-servers.md) - MCP client remote server registration, CRUD API, tool naming, execution.
* [Runtime MCP Client Specification](runtime-mcp.md) - MCP client in the in-process runtime: shared core MCP module, transport abstraction (HTTP + optional stdio), pluggable auth.
* [Agent MCP Attachments (acts-as semantics)](agent-mcp-attachments.md) - Make who an MCP server acts as an explicit, fail-closed property of an Agent attachment; org MCP servers become presets; one MCP surface per Agent.
* [User MCP servers and agent MCP auth modes](user-mcp-servers.md) - Virtual users own MCP servers; agents opt in to use or manage them; agent-level servers choose service, user, or user-with-service-fallback auth; connect from chat; one mechanism shared with yolop.
* [User Connections](user-connections.md) - External accounts (GitHub, GitLab, Daytona) linked to a user, resolved lazily at tool execution.
* [MCP Events: session webhooks out, agent triggers in](mcp-events.md) - /mcp clients subscribe to session webhooks; agents subscribe to their MCP servers' events as `mcp_event` triggers.
* [Inbound Form Mode Elicitation](mcp-form-elicitation.md) - Answering an attached MCP server's form mode elicitation through ask_user, and the trust rules that shape it.
* [Integrations](integrations.md) - Integration specs index.
* [Archived Apps](apps.md) - Frozen App compatibility data and permanent ingress aliases.
* [Agent Exposure (retiring the App abstraction)](agent-exposure.md) - Make Agent the addressable entity by making channels Agent-owned and folding invocation into Triggers, retiring App.
* [Public Chat (Hosted Chat App)](public-chat.md) - Public Chat (hosted, isolated chat app), product spec/proposal.
* [Legacy App Invocation Aliases](app-invocation-channels.md) - Frozen App-shaped aliases for channel-owned webhook and schedule ingress.
* [Channel Authentication](channel-auth.md) - Shared inbound auth framework for Agent-owned channels.
* [AgentID](agentid.md) - AgentID (OIDC for AI agents): channel preset, consumer sign-in, and agents finishing other apps' AgentID sign-ins.
* [Agent Execution API](agent-execution-api.md) - Proposal: expose one agent to code with an API channel, agent keys, customer OAuth, session and run routes, shared with serve; the SDK becomes the agent client.
* [Legacy App API Keys](app-api-keys.md) - Frozen execution-only credentials for channel-owned native session ingress.
* [AG-UI Channel](ag-ui.md) - AG-UI 1.0 channel: wire types, runtime-event projection, the consumer pipeline, and the 1.0 rules each side keeps.
* [A2A Channel](a2a-channel.md) - A2A inbound channel.
* [A2A Capability](a2a-capability.md) - A2A outbound delegation capability.
* [AG-UI Capability](ag-ui-capability.md) - AG-UI outbound delegation: configured external AG-UI agents as spawn_agent targets backed by session tasks.
* [FCP (Free Communication Protocol) channel](fcp-channel.md) - FCP inbound channel.
* [Channels](channels.md) - One channel implementation for the Framework, serve and the server: definitions, platform drivers and one channel host in core.
* [Messaging Integrations](messaging-integrations.md) - Messaging integrations.
* [Explicit Communication](explicit-communication.md) - Proposal: an agent setting where assistant text stays private and the agent talks only through send_message and related tools, with framed inbound messages.
* [Slack Bot Integration](slack-integration.md) - Slack channel: per-agent Slack app, webhook flow, session routing, delivery, security review.
* [Slack Integration Modernization](slack-modernization.md) - Gap analysis of the Slack channel against the current Slack agent platform, with a prioritized set of improvements.
* [Slack Agent Actions](slack-agent-actions.md) - Why Slack approvals, task progress, and the second-identity problem are one missing capability.
* [Slack One-Click Install](slack-one-click-install.md) - What a live PoC established about creating per-agent Slack apps programmatically.
* [Per-agent GitHub Apps](github-apps.md) - One-click GitHub App per agent identity: one installation for GitHub tools, MCP and events.
* [GitHub review and security agent templates](github-agent-templates.md) - PR Reviewer and Security Scanner: guided agent examples, deterministic repeat suppression, settings the tools enforce.
* [Modal Sandboxes](modal.md) - Modal VM and gVisor sandboxes as the first module of the everruns-integrations crate: gRPC transport, credentials, state and leases, testing.
* [Plugins](plugins.md) - Plugin host: marketplaces and cross-host plugin packages installed as capabilities.
* [Model Router Specification](model-router.md) - Model Routers.
