---
title: MCP OAuth keeps its audience when tokens refresh
description: MCP connections preserve the authorized resource through token refresh and restart, and older grants request a new login instead of guessing an audience.
date: 2026-11-05
kind: fix
significance: minor
products: [platform, framework]
surfaces: [api, code]
---

MCP OAuth now retains the resource selected during sign-in when it saves and
refreshes credentials, including refresh-token rotation and process restarts.
Servers without protected-resource metadata use the validated MCP URL during
sign-in. Changes to a platform connection's OAuth settings require reconnecting.

Older credentials without a stored resource continue using their current access
token until refresh is needed, then require a new sign-in. Reconnect the MCP
server when prompted. This strengthens audience binding; it does not establish
that an authorization server issued tokens for an unintended audience.
