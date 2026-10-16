---
title: Docker Compose forwards OpenRouter inference keys
description: The self-hosted Compose stack now accepts an OpenRouter inference key from its environment, with updated provider setup and isolated real-provider testing.
date: 2026-10-16
kind: fix
significance: minor
products: [platform]
surfaces: [operations]
links:
  docs: [/getting-started/docker-compose/]
---

The Docker Compose stack now forwards `DEFAULT_OPENROUTER_API_KEY` to the control
plane. Previously, adding this variable to the quickstart's `.env` file left the
OpenRouter provider without a key. Use an inference key rather than a management
key, or configure the provider through **Models → Providers**.

The [Docker Compose guide](/getting-started/docker-compose/) also documents both
host-port overrides for running isolated copies, the Compose version required by
inline configuration, and the difference between the downloaded file and
published images. An opt-in real-provider E2E records startup, provider setup,
agent creation through Chat, and a successful Playground run; its separate
startup-only mode makes no inference calls.
