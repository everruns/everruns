---
title: Agents no longer lose MCP servers silently when sign-in expires
description: The agent's MCP sheet now shows whether each sign-in really works, Health warns before one stops, and the agent is told when a server is gone.
date: 2026-11-22
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/mcp/]
---

An agent that signs in to an OAuth MCP server as itself could lose that server without anyone noticing. Once the sign-in expired and could not renew, the server's tools dropped out of every turn while the agent's MCP sheet still said **Connected**.

Now the sheet, renamed **MCP**, shows whether each sign-in works and renews, stops on a given date, or **stopped working**, with a **Sign in again** button. Health warns three days ahead, and `everruns agents mcp list` shows the same states. Until someone signs in again, the agent gets one `sign_in_required` tool, so it can say what is missing. See [MCP](/features/mcp/).

Sign-ins made before November 5 cannot renew themselves and need signing in again once.
