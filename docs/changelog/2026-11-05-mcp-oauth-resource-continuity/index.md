---
title: MCP OAuth keeps its audience when tokens refresh
description: MCP connections preserve the authorized resource through token refresh and restart, and older grants request a new login instead of guessing an audience.
date: 2026-11-05
release: "0.51.0"
kind: fix
significance: minor
products: [platform, framework, yolop]
surfaces: [api, code, cli]
links:
  prs: [everruns/everruns#5239, everruns/yolop#916]
---

MCP OAuth now retains the resource selected during sign-in when it saves and
refreshes credentials, including refresh-token rotation and process restarts.
Servers without protected-resource metadata use the validated MCP URL during
sign-in. Changes to a platform connection's OAuth identity require reconnecting before
its next refresh.

Older credentials without a stored resource continue using their current access
token until refresh is needed, then require a new sign-in. Reconnect the MCP
server when prompted. This strengthens audience binding; it does not establish
that an authorization server issued tokens for an unintended audience.

[Yolop 0.23.2](https://github.com/everruns/yolop/releases/tag/v0.23.2)
also preserves the resource in its saved MCP credentials, including repeated
refresh-token rotation and restarts. For an older Yolop connection, run
`yolop mcp login <name>` when refresh requests a new sign-in, then
`yolop mcp test <name>` to verify discovery.

## Update 2026-11-06

Linux download users should use [Yolop 0.23.3](/changelog/2026-11-06-yolop-linux-downloads/),
which restores glibc 2.39 compatibility after a packaging regression in 0.23.2.
