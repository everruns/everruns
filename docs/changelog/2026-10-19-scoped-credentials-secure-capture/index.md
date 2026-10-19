---
title: Credentials work across users, agents and sessions
description: Bind write-only credentials to a user, agent, session or organization, grant agents explicit access, and capture API tokens with exact-request consent.
date: 2026-10-19
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api]
links:
  docs:
    - /capabilities/web-fetch/
    - /integrations/mcp-credentials/
---

Credentials can now belong to a virtual user, agent, session or organization. Open **Credentials** on an agent or session, or manage personal and organization credentials in Settings. Organization credentials require an explicit access grant. Values stay write-only; revoke a binding or grant to stop its next participating request.

![Organization credentials with explicit agent access](./credentials@2x.png)

[Web Fetch](/capabilities/web-fetch/) can capture several JSON response fields directly into encrypted storage. Persistent capture asks for approval of the exact request, destination and acting identity. Subsequent Web Fetch and Bashkit HTTP calls use references such as `$MAIL_TOKEN`; injected values and recognized response echoes are removed before tool results reach the agent.

The same ownership and access rules apply to [MCP parameter credentials](/integrations/mcp-credentials/). Existing GitHub connections retain their installation selection and repository limits.

**API change:** credential value updates now require the observed `revision` to prevent stale writes.
