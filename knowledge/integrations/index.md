# MCP, integrations, and apps

* [MCP (Model Context Protocol) Specification](mcp.md) - MCP server endpoint, OAuth 2.1 authentication, protocol, security.
* [MCP Server Specification](mcp-servers.md) - MCP client remote server registration, CRUD API, tool naming, execution.
* [Runtime MCP Client Specification](runtime-mcp.md) - MCP client in the in-process runtime: shared `everruns-mcp` crate, transport abstraction (HTTP + optional stdio), pluggable auth.
* [Agent MCP Attachments (acts-as semantics)](agent-mcp-attachments.md) - Make who an MCP server acts as an explicit, fail-closed property of an Agent attachment; org MCP servers become presets; one MCP surface per Agent.
* [Integrations](integrations.md) - Integration specs index.
* [Apps](apps.md) - Frozen App compatibility data and permanent ingress aliases.
* [Agent Exposure (retiring the App abstraction)](agent-exposure.md) - Make Agent the addressable entity by re-homing App channels as agent-owned Channels and folding invocation into Triggers, retiring App.
* [Public Chat (Hosted Chat App)](public-chat.md) - Public Chat (hosted, isolated chat app), product spec/proposal.
* [App Invocation Channels](app-invocation-channels.md) - App schedule/webhook invocation channels.
* [Channel Authentication](channel-auth.md) - Shared inbound auth framework for Agent-owned channels.
* [Legacy App API Keys](app-api-keys.md) - Frozen execution-only credentials for channel-owned native session ingress.
* [A2A Channel](a2a-channel.md) - A2A inbound channel.
* [A2A Capability](a2a-capability.md) - A2A outbound delegation capability.
* [FCP (Free Communication Protocol) channel](fcp-channel.md) - FCP inbound channel.
* [Messaging Integrations](messaging-integrations.md) - Messaging integrations.
* [Slack Integration Modernization](slack-modernization.md) - Gap analysis of the Slack channel against the current Slack agent platform, with a prioritized set of improvements.
* [Slack Agent Actions](slack-agent-actions.md) - Why Slack approvals, task progress, and the second-identity problem are one missing capability.
* [Slack One-Click Install](slack-one-click-install.md) - What a live PoC established about creating per-agent Slack apps programmatically.
* [Plugins](plugins.md) - Plugin host: marketplaces and cross-host plugin packages installed as capabilities.
* [Model Router Specification](model-router.md) - Model Routers.
