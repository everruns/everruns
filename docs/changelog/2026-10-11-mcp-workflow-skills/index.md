---
title: Load Everruns workflow skills through MCP
description: The Everruns MCP endpoint serves two workflow skills for running sessions and authoring agents, with metadata and verified file manifests.
date: 2026-10-11
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/features/mcp/]
  prs: [everruns/everruns#4578]
---

Hosts supporting the MCP Skills extension can discover and load workflow
guidance directly from the Everruns endpoint. Use `skills/list` under MCP
`2026-07-28`, then load a skill through your host's skill loader.
`everruns-mcp` guides platform and session work; `everruns-agent-authoring`
guides creating and revising agents.

Each entry includes a complete file manifest with SHA-256 digests. The endpoint
serves only these two workflows; the Rust Framework skill remains separately
installable. See [Skills for MCP clients](/features/mcp/#skills-for-mcp-clients)
for discovery, file reads, and host verification.
