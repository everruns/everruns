---
title: Environments
description: Configure where an Agent runs commands, select that Environment for a chat, and recover durable workspace state after compute loss.
appliesTo: [platform, cloud]
---

An **Environment** is the filesystem and compute target used by a session's command tools. It is
separate from the Harness: the Harness defines behavior and capabilities, while the Environment
selects Bashkit, Daytona, or another execution target without changing the tool names the model
uses.

## Configure an Agent

1. Open **Agents**, then open the Agent.
2. Under **More**, select **Environments**.
3. Select **Add Bashkit** or **Add Daytona**.
4. Give the profile a stable name such as `scratch` or `build`, and choose which profile is the
   default.
5. For Daytona, optionally select compute size, snapshot, workspace path, provider auto-stop, idle
   behavior, and bootstrap commands.
6. Select **Done**, then **Save changes** on the Agent page.

Bashkit is available without a provider connection. Daytona appears as unavailable when its
provider integration is not installed in the deployment. A user who starts a Daytona chat also
needs a Daytona connection under **Settings > Connections**; the session returns an actionable
connection error when that user has not connected it.

Do not put API keys or other credentials in target options or bootstrap commands. Bind provider
credentials through the deployment's connection management instead.

## Start a chat in an Environment

1. Open **Chats** and select **New chat**.
2. Select the Agent.
3. Select one of the Agent's named **Environment** profiles. The Agent default is preselected.
4. Select **Start chat**.

The new session pins a resolved snapshot of that profile. Editing the Agent later affects new
sessions only; an existing chat does not silently move to a different target or policy.

Open the session's **Workspace** tab to inspect the resolved target, containment, capabilities,
recovery class, and latest lifecycle state.

## Recovery after compute loss

A `checkpointed` profile separates durable workspace state from replaceable physical compute.
Everruns checkpoints successful mutations. If a Daytona sandbox physically disappears, the next
environment operation detects the loss, creates replacement compute, restores the workspace, and
continues the same session.

Files under the managed workspace survive that replacement. Running processes, open terminals,
and in-memory process state do not. Lifecycle events report both the lost instance and the recovery
so operators can distinguish restored files from restored processes.

## API

Create or update an Agent with named profiles:

```json
{
  "environments": {
    "default": "scratch",
    "profiles": {
      "scratch": {
        "target": { "kind": "vfs", "provider": "bashkit" }
      },
      "build": {
        "target": { "kind": "managed", "provider": "daytona" },
        "durability": "checkpointed"
      }
    }
  }
}
```

Select a named profile when creating a session:

```json
{
  "agent_id": "agent_...",
  "environment": { "use": "build" }
}
```

Use `GET /v1/environment-targets` to discover what the deployment can actually run, and
`GET /v1/sessions/{session_id}/environment` to inspect the immutable resolved profile and current
lifecycle state.
