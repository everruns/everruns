---
title: Integrations
description: Connect Everruns agents to cloud sandboxes, browsers, search providers, and messaging channels. Integrations are auto-registered and surface as agent capabilities.
sidebar:
  label: Overview
  order: 0
---

Integrations connect Everruns agents to external services — cloud sandboxes, browsers, search providers, and messaging channels. Each is auto-registered via the `inventory` plugin system and, once a connection is configured, surfaces to agents as a [capability](/features/capabilities/) or [App channel](/features/apps/).

## Sandboxes & execution

Give agents an isolated environment to run code, edit files, and persist state.

| Integration | What it provides |
|---|---|
| [Daytona](/integrations/daytona/) | Cloud sandbox environments via the Daytona REST API |
| [E2B](/integrations/e2b/) | Cloud sandboxes via the E2B management + runtime APIs (bring your own key) |
| [Container Sandbox](/integrations/container-sandbox/) | Self-hosted container sandboxes via Docker Engine — no external SaaS |
| [Deno](/integrations/deno/) | Cloud sandboxes via the Deno websocket sandbox API |
| [Sprites](/integrations/sprites/) | Persistent Firecracker microVMs with checkpoints and HTTP services |
| [Cursor](/integrations/cursor/) | Launch and manage asynchronous Cursor Cloud coding agents |

## Browser & web

| Integration | What it provides |
|---|---|
| [Browserless](/integrations/browserless/) | Cloud browser automation — screenshots, DOM, scraping, multi-step flows |
| [Brave Search](/integrations/brave-search/) | Web search via the Brave Search API |
| [DuckDuckGo](/integrations/duckduckgo/) | Instant answers via the DuckDuckGo API |
| [Parallel](/integrations/parallel/) | Web search, extract, and task APIs (free and paid tiers) |

## Messaging channels

| Integration | What it provides |
|---|---|
| [Slack](/integrations/slack/) | Deploy an agent as a Slack bot via an [App](/features/apps/) |

## Discovery

| Integration | What it provides |
|---|---|
| [ARD](/integrations/ard/) | Client-side discovery of external MCP servers and A2A agents at runtime |

## Model providers

Integrations connect agents to tools and services. **[Providers](/providers/)**
connect Everruns to the AI model vendors that run your agents — OpenAI,
Anthropic, Google Gemini, AWS Bedrock, OpenRouter, and more. See the
[Providers overview](/providers/) to configure one.

## Adding an integration

New integrations follow a parity checklist (connection provider, tests, live-API coverage, docs, and a threat-model section) before they ship. Daytona is the reference implementation. See the in-repo [`specs/integrations.md`](https://github.com/everruns/everruns/blob/main/specs/integrations.md) for the full contract.
