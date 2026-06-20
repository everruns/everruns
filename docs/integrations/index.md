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
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" d="M12 2.5 21 7v10l-9 4.5L3 17V7z"/><path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" d="M3 7l9 4.5L21 7"/><path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" d="M12 11.5V21.5"/></svg>[Daytona](/integrations/daytona/) | Cloud sandbox environments via the Daytona REST API |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><rect x="2.5" y="4" width="19" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M6.5 9.5l3 2.5-3 2.5M12.5 15h5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>[E2B](/integrations/e2b/) | Cloud sandboxes via the E2B management + runtime APIs (bring your own key) |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><rect x="3" y="13" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="1.6"/><rect x="11" y="13" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="1.6"/><rect x="7" y="5.5" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="1.6"/></svg>[Container Sandbox](/integrations/container-sandbox/) | Self-hosted container sandboxes via Docker Engine — no external SaaS |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><path d="M12 2.5l8.5 4.9v9.2L12 21.5l-8.5-4.9V7.4z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/><circle cx="12" cy="12" r="3.1" fill="none" stroke="currentColor" stroke-width="1.6"/></svg>[Deno](/integrations/deno/) | Cloud sandboxes via the Deno websocket sandbox API |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><path d="M12 2.5c2.2 3 1.1 4.8-.5 6.2.9-3.2-2.6-3.7-2.6-3.7C7 8.3 6 10.2 6 12.7a6 6 0 0 0 12 0c0-2.1-1-4.2-2.6-5.7.3 2.1-1.2 3-1.2 3 .6-2.4-.8-5.5-2.2-7.5z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>[Sprites](/integrations/sprites/) | Persistent Firecracker microVMs with checkpoints and HTTP services |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><path d="M5 3l14 7-6 2.2L10.5 19z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>[Cursor](/integrations/cursor/) | Launch and manage asynchronous Cursor Cloud coding agents |

## Browser & web

| Integration | What it provides |
|---|---|
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><rect x="2.5" y="4" width="19" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M2.5 8.5h19" fill="none" stroke="currentColor" stroke-width="1.6"/><circle cx="5.6" cy="6.25" r=".75" fill="currentColor"/><circle cx="7.8" cy="6.25" r=".75" fill="currentColor"/></svg>[Browserless](/integrations/browserless/) | Cloud browser automation — screenshots, DOM, scraping, multi-step flows |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M21 21l-5.4-5.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>[Brave Search](/integrations/brave-search/) | Web search via the Brave Search API |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M9.4 9.1a2.6 2.6 0 0 1 5 .9c0 1.8-2.4 2.1-2.4 3.6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="17" r=".75" fill="currentColor"/></svg>[DuckDuckGo](/integrations/duckduckgo/) | Instant answers via the DuckDuckGo API |
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><path d="M7 4v16M12 4v16M17 4v16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>[Parallel](/integrations/parallel/) | Web search, extract, and task APIs (free and paid tiers) |

## Messaging channels

| Integration | What it provides |
|---|---|
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><path d="M9 3.5L7 20.5M17 3.5l-2 17M4 8.5h16M3.2 15.5h16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>[Slack](/integrations/slack/) | Deploy an agent as a Slack bot via an [App](/features/apps/) |

## Discovery

| Integration | What it provides |
|---|---|
| <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" style="vertical-align: -0.2em; margin-right: 0.45em;"><circle cx="12" cy="5" r="2.3" fill="none" stroke="currentColor" stroke-width="1.6"/><circle cx="5" cy="18" r="2.3" fill="none" stroke="currentColor" stroke-width="1.6"/><circle cx="19" cy="18" r="2.3" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M10.7 6.9L6.3 15.9M13.3 6.9L17.7 15.9M7.3 18h9.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>[ARD](/integrations/ard/) | Client-side discovery of external MCP servers and A2A agents at runtime |

## Model providers

Integrations connect agents to tools and services. **[Providers](/providers/)**
connect Everruns to the AI model vendors that run your agents — OpenAI,
Anthropic, Google Gemini, AWS Bedrock, OpenRouter, and more. See the
[Providers overview](/providers/) to configure one.

## Adding an integration

New integrations follow a parity checklist (connection provider, tests, live-API coverage, docs, and a threat-model section) before they ship. Daytona is the reference implementation. See the in-repo [`specs/integrations.md`](https://github.com/everruns/everruns/blob/main/specs/integrations.md) for the full contract.
