---
title: User MCP management respects approval settings
description: Adding and enabling personal MCP servers now follow the agent's shared Tool Approval mode, preserving URL restrictions and separate OAuth consent.
date: 2026-10-16
cloud: 2026-10-17
kind: fix
significance: minor
products: [platform, cloud, framework]
release: "0.49.0"
surfaces: [agent, code]
links:
  docs: [/capabilities/user-mcp-servers/, /capabilities/tool-approval/]
---

Agents with [User MCP Servers](/capabilities/user-mcp-servers/) management
enabled now honor the shared [Tool Approval](/capabilities/tool-approval/)
settings when adding or enabling a server. Normal and protective modes ask
first. With Tool Approval absent or set to `off`, management runs without a
hidden approval gate.

Agents with management disabled also stop advertising management tools to the
model. Management still acts only for the person chatting. Catalog restrictions,
custom-URL validation and separate OAuth consent continue to apply.
