---
title: Framework integrations
description: Every integration a Framework app can use, the Cargo feature that turns it on, and where its documentation lives.
---

These integrations run inside your own Framework application. Turn on the
listed Cargo feature (on the `everruns` crate unless the row names another),
then add the capability to the agent.
Turning a feature on compiles the integration in, but no agent gets it until you
add the capability. Feature and boundary details are in
[Capability integrations](/framework/capability-integrations/).

```toml
[dependencies]
everruns = { version = "0.52", features = ["openai", "web-search", "web-fetch"] }
```

## Search and web

| Integration | Cargo feature | Docs |
|---|---|---|
| Web search: Brave, Parallel, provider-native search, or a free fallback | `web-search` | [Web Search](/capabilities/web-search/#framework) |
| Brave Search as its own typed capability | `everruns-integrations` crate, `brave-search` feature | [Provider integrations](/framework/capability-integrations/#provider-integrations) |
| DuckDuckGo instant answers | `duckduckgo` | [DuckDuckGo](/integrations/duckduckgo/) |
| Web fetch | `web-fetch` | [Web Fetch](/capabilities/web-fetch/) |

## Code and files

| Integration | Cargo feature | Docs |
|---|---|---|
| Session filesystem | `filesystem` (default) | [File System](/capabilities/file-system/) |
| Bashkit sandboxed shell | `bashkit` | [Bashkit Shell](/capabilities/bashkit-shell/) |
| Shell on the host machine | `host-shell` | [Host Shell](/capabilities/host-shell/) |
| Lua sandbox | `lua` | [Capability integrations](/framework/capability-integrations/#framework-features) |

## Models, media and judgment

| Integration | Cargo feature | Docs |
|---|---|---|
| Model providers: OpenAI, Anthropic, Gemini, Bedrock, OpenRouter and more | one feature per provider | [Models and Providers](/framework/models-and-providers/) |
| Image generation: stills, GIFs and SVGs | `images` | [Image Generation](/capabilities/image-generation/) |
| TypeSafe typed decisions | `typesafe` | [TypeSafe](/integrations/typesafe/#embedding-the-framework) |

## Tools and other agents

| Integration | Cargo feature | Docs |
|---|---|---|
| MCP servers over HTTP | `mcp` | [MCP and plugins](/framework/agents/#mcp-and-plugins) |
| Local-process (stdio) MCP servers | `mcp-stdio` | [MCP and plugins](/framework/agents/#mcp-and-plugins) |
| Delegate to remote A2A agents | `a2a` | [Call remote A2A agents](/framework/a2a/#call-remote-a2a-agents) |
| Personal agent (Poppy) | `poppy` | [Poppy in the Framework](/capabilities/poppy-personal-agent/#in-the-framework) |

## Put an agent in front of people

| Integration | Cargo feature | Docs |
|---|---|---|
| Serve over A2A | `everruns-serve` crate, `a2a` feature | [Serve your agents over A2A](/framework/a2a/#serve-your-agents-over-a2a) |
| AG-UI frontends | `ag-ui`, `ag-ui-axum` | [Serve AG-UI](/framework/ag-ui/) |
| Slack and webhook channels | `channels` | [`everruns::channels` API docs](https://docs.rs/everruns/latest/everruns/channels/) |
| Voice through a speech-to-speech provider | `voice` | [Build a Voice Agent](/framework/voice/) |

## Observability

| Integration | Cargo feature | Docs |
|---|---|---|
| OpenTelemetry | `otel` | [OpenTelemetry](/framework/observability/#opentelemetry) |
| Braintrust | `braintrust` | [Braintrust](/framework/observability/#braintrust) |

## Platform only

The integrations below run in a Platform deployment, where the server holds the
connection and its credentials: cloud sandboxes (Daytona, E2B, Container
Sandbox, Modal, boxd, Sprites), Browserless, Cursor, GitHub, Cloudflare Web
Search, the Parallel tool set, the hosted Slack app, AgentID and ARD. A Framework
app reaches Parallel through [web search](/capabilities/web-search/#framework).
To use any of the others, [move to the Platform](/framework/moving-to-platform/).
The full catalog is in [Integrations](/integrations/).
