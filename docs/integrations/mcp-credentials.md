---
title: Secure MCP Credentials
description: Configure write-only Agent credentials for MCP tools that require a secret parameter.
---

Use an Agent credential binding when an MCP tool requires a secret in its input,
such as Visti's `visti_send.channel_key`. Do not paste the value into chat,
Agent instructions, memory, or session storage.

1. Attach the MCP server capability to the Agent.
2. Open the Agent and select **Credentials**.
3. Add or open the exact server, tool, and parameter binding.
4. Enter the value in the masked form and save it.

The value is encrypted and is never shown again. Everruns removes the bound
parameter from the model-visible tool schema and injects the value only when it
sends the MCP request. The same Agent binding works for a shared session and
for triggers that create a new session per invocation.

Use **Rotate** to replace a value. Use the revoke action to delete the binding;
future calls then return a setup-required result with a link back to the
agent's Credentials settings.

Session Storage has a separate encrypted secret lifecycle for session-local
workflows. Those secrets do not follow per-invocation sessions, and a model can
read them with `secret_store get`, so they are not a substitute for an MCP
credential binding.

## OAuth connections

For an MCP attachment that acts as a user, connect the provider from **Settings → My agent experience**. The grant belongs to your current organization's virtual user and is used only for turns you initiate. Scheduled execution does not borrow the last speaker's grant.

For an attachment that acts as a service, select the responding agent's service virtual user and connect the provider on that account's **Connections** tab. Agent behavior and service credentials remain separate.

External integrations can exchange verified channel authentication at `POST /v1/channels/{channel_id}/runtime-auth` for a short-lived runtime bearer token. The token allows virtual-user self-service and authenticated ingress for that channel. It cannot administer organizations, agents, or other users. Use `GET /v1/virtual-users/me` to discover the account, and the canonical `/v1/virtual-users/{id}/connections` routes for its grants. To start browser OAuth with bearer authentication, POST to the provider's `authorize` subresource and navigate to the returned `authorization_url` while retaining its setup cookie. Setup captures the account and revalidates authority at callback.
