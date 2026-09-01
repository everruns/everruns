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

Platform Chat can also provision a value it was already given, so you do not
have to re-enter a key you already sent it. It never asks for one: if you have
not supplied a value, it hands you the secure setup link instead. A value sent
through chat stays in that conversation's transcript, so rotate it if the
transcript is kept somewhere the credential should not be.

The value is encrypted and is never shown again. Nothing reads it back:
no API or agent command returns a stored value. Everruns removes the bound
parameter from the model-visible tool schema and injects the value only when it
sends the MCP request. The same Agent binding works for a shared session and
for triggers that create a new session per invocation.

Use **Rotate** to replace a value. Use the revoke action to delete the binding;
future calls then return a setup-required result with a link back to the
Credentials tab.

Session Storage has a separate encrypted secret lifecycle for session-local
workflows. Those secrets do not follow per-invocation sessions, and a model can
read them with `secret_store get`, so they are not a substitute for an MCP
credential binding.
