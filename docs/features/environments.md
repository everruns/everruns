---
title: Environments
description: Configure where an Agent runs commands, select that Environment for a chat, and recover durable workspace state after compute loss.
appliesTo: [platform, cloud]
---

An **Environment** is reusable, versioned configuration for filesystem and compute. A session pins
one immutable Environment revision into its **primary Sandbox**. The Sandbox is the durable logical
resource; its physical Daytona instance or other provider compute may be replaced without changing
the session or Sandbox identity.

Environment and Harness are separate. A Harness defines behavior and can either fix an Environment
or let an Agent choose one. Environment revisions select Bashkit, Daytona, or another execution
target without changing the ordinary shell and file tools the model uses.

## Create an Environment

1. Open **Environments** and select **New environment**.
2. Give it a stable name and display name.
3. Select Bashkit Virtual Workspace or an available managed provider such as Daytona.
4. Configure durability, lifecycle, and bootstrap options, then save.

Editing an Environment creates a new revision. Existing Agents and sessions keep the revision they
already reference; new bindings can use the new current revision. Managed Environments such as the
Bashkit Virtual Workspace are provisioned by Everruns and cannot be edited or archived.

## Configure an Agent

1. Open **Agents**, then open the Agent.
2. Under **More**, select **Environments**.
3. Select an existing Environment. The Agent stores its exact revision, not a moving pointer.
4. Choose the Agent policy:
   - **Fixed**: the session always gets the single configured Environment and cannot override it.
   - **Selectable**: Playground can choose from the Agent's declared Environments.
   - **Configurable**: Playground may also provide an inline Environment profile.
5. Choose the default Environment when the policy permits multiple choices.
6. Select **Done**, then **Save changes** on the Agent page.

Bashkit is available without a provider connection. Daytona appears as unavailable when its
provider integration is not installed in the deployment. A user who starts a Daytona chat also
needs a Daytona connection under **Settings → My agent experience**; the session returns an actionable
connection error when that user has not connected it.

Do not put API keys or other credentials in target options or bootstrap commands. Bind provider
credentials through the deployment's connection management instead.

The built-in [Bashkit Worker](/built-ins/harnesses/bashkit-worker/) is sealed to Everruns' managed
Bashkit Environment. Agents based on it cannot configure Environment profiles, and session creation
cannot override the primary Sandbox. Use provider-neutral Worker or Worker Base when the Agent must
select Daytona or another Environment.

## Start a Playground session in an Environment

1. Open **Playground** and start a new session.
2. Select the Agent.
3. For a selectable or configurable Agent, select an allowed **Environment**. Fixed Agents show the
   resolved Environment as read-only.
4. Select **Start chat**.

The new session pins a resolved snapshot of that profile. Editing the Agent later affects new
sessions only; an existing chat does not silently move to a different target or policy.

Open the session's **Workspace** tab to inspect the primary Sandbox id, pinned Environment revision,
resolved target, containment, capabilities, recovery class, generation, and latest lifecycle state.

## Recovery after compute loss

A `checkpointed` Environment separates durable workspace state from replaceable physical compute.
Everruns checkpoints successful mutations. If a Daytona sandbox physically disappears, the next
environment operation detects the loss, creates replacement compute, restores the workspace, and
continues the same session.

Files under the managed workspace survive that replacement. Running processes, open terminals,
and in-memory process state do not. Lifecycle events report both the lost instance and the recovery
so operators can distinguish restored files from restored processes.

## API

Create a reusable Environment:

```json
POST /v1/environments
{
  "name": "daytona-build",
  "display_name": "Daytona Build",
  "profile": {
    "target": { "kind": "managed", "provider": "daytona" },
    "durability": "checkpointed"
  }
}
```

Then bind exact Environment revisions to an Agent:

```json
{
  "environments": {
    "policy": "selectable",
    "default": "scratch",
    "profiles": {
      "scratch": {
        "source_revision_id": "envrev_...",
        "target": { "kind": "vfs", "provider": "bashkit" }
      },
      "build": {
        "source_revision_id": "envrev_...",
        "target": { "kind": "managed", "provider": "daytona" },
        "durability": "checkpointed"
      }
    }
  }
}
```

Select a declared Environment only in Playground and only when the policy is `selectable` or
`configurable`:

```json
{
  "agent_id": "agent_...",
  "environment": { "use": "build" }
}
```

Use `GET /v1/environment-targets` to discover what the deployment can actually run, and
`GET /v1/sessions/{session_id}/environment` to inspect the immutable resolved profile and current
lifecycle state. Use `GET /v1/environments` to list reusable definitions.

## Resource Sandbox fleets

The primary Sandbox is implicit: ordinary shell and file tools always address it. An Agent with the
`sandbox_fleet` capability can additionally create explicitly addressed resource Sandboxes through
`sandbox_create`, then use `sandbox_exec`, `sandbox_read_file`, `sandbox_write_file`,
`sandbox_inspect`, `sandbox_checkpoint`, and `sandbox_manage`. Resource Sandbox IDs are logical,
session-scoped handles; provider IDs and credentials remain inside the trusted control plane. Fleet
operations never change where ordinary shell or file tools run.
