---
title: Credentials work across users, agents and sessions
description: Bind write-only credentials to a user, agent, session or organization, grant agents explicit access, and capture API tokens with exact-request consent.
date: 2026-10-19
kind: feature
significance: major
products: [platform, cloud, framework]
release: "0.49.0"
surfaces: [ui, api, cli]
links:
  docs:
    - /capabilities/web-fetch/
    - /integrations/mcp-credentials/
---

Credentials can belong to virtual users, agents, sessions or organizations. Open **Credentials** on an agent or session, or manage personal and organization credentials in Settings. Organization access requires explicit grants. Values stay write-only; revocation blocks future participating requests.

![Organization credentials with explicit agent access](./credentials@2x.png)

[Web Fetch](/capabilities/web-fetch/) can capture several JSON response fields directly into encrypted storage. Persistent capture asks for approval of the exact request, destination and acting identity. Subsequent Web Fetch and Bashkit HTTP calls use references such as `$MAIL_TOKEN`; injected values and recognized response echoes are removed before tool results reach the agent.

The same ownership and access rules apply to [MCP parameter credentials](/integrations/mcp-credentials/). Unattended user OAuth keeps its sign-in guidance. Existing GitHub connections retain their installation selection and repository limits.

The [CLI credential commands](/reference/cli/credential-bindings/) manage metadata and revocation; values are entered through the secure Credentials form.

**API change:** credential value updates now require the observed `revision` to prevent stale writes.
