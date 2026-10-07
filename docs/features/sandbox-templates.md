---
title: Sandbox Templates
description: Configure reusable sandbox recipes, bind one to an Agent, and recover Session workspace state after compute loss.
appliesTo: [platform, cloud]
---

A **Sandbox Template** is reusable, versioned configuration for filesystem and compute. A new
Session pins one immutable template revision into its **primary Sandbox**. The Sandbox is the
durable logical resource; its physical Daytona instance or other provider compute may be replaced
without changing the Session or Sandbox identity.

Sandbox Templates and Harnesses are separate. A Harness defines behavior and may fix the Sandbox
policy or let an Agent offer template bindings. A template selects Bashkit, Daytona, or another
execution target without changing the ordinary shell and file tools the model uses.

## Create a Sandbox Template

1. Open **Sandbox Templates** and select **New Sandbox Template**.
2. Give it a stable name and display name.
3. Select Bashkit or an available managed provider such as Daytona.
4. Configure durability, lifecycle, and bootstrap options, then save.

Editing a Sandbox Template creates a new immutable revision. Existing Sessions keep the revision
they already reference. Managed templates such as Bashkit Virtual Workspace are
provisioned by Everruns and cannot be edited or archived.

## Configure an Agent

1. Open **Agents**, then open the Agent.
2. Under **More**, select **Primary sandbox**.
3. Add a Sandbox Template. The Agent stores its exact revision, not a moving pointer.
4. Choose the policy mode:
   - **Fixed**: every Session gets the single configured Sandbox; Session overrides are rejected.
   - **Selectable**: Playground can select one of the Agent's declared template bindings.
   - **Configurable**: Playground may also submit constrained one-off configuration.
5. Choose the default binding when the policy permits multiple choices.
6. Select **Done**, then **Save changes**.

Bashkit is available without a provider connection. Daytona is shown as unavailable when its
integration is not installed. A user who starts a Daytona Session also needs a Daytona connection
under **Settings → My agent experience**. Do not put credentials in target options or bootstrap
commands; use connection management instead.

The built-in [Bashkit Worker](/built-ins/harnesses/bashkit-worker/) seals the primary Sandbox to
Everruns' managed Bashkit template. Agents based on it cannot change the policy, and Session
creation cannot override it. Use provider-neutral Worker or Worker Base when an Agent must select
Daytona or another target.

## Start a Playground Session

1. Open **Playground** and start a new Session.
2. Select the Agent.
3. For a selectable or configurable Agent, select an allowed **Sandbox**. A fixed Agent shows the
   resolved Sandbox as read-only.
4. Select **Start Playground chat**.

The Session pins the resolved specification. Editing the Agent or Sandbox Template later affects
new Sessions only. Open the Session's **Workspace** tab to inspect the primary Sandbox ID, pinned
template revision, target, containment, capabilities, recovery class, generation, and lifecycle
state.

## Recovery after compute loss

A `checkpointed` Sandbox separates durable workspace state from replaceable physical compute.
Everruns checkpoints successful mutations. If a Daytona instance disappears, the next sandbox
operation creates replacement compute, restores the workspace, and continues the same Session.

Files under the managed workspace survive replacement. Running processes, open terminals, and
in-memory process state do not.

## API

Create a reusable template:

```json
POST /v1/sandbox-templates
{
  "name": "daytona-build",
  "display_name": "Daytona Build",
  "spec": {
    "target": { "kind": "managed", "provider": "daytona" },
    "durability": "checkpointed"
  }
}
```

Bind exact revisions to an Agent:

```json
{
  "sandbox_policy": {
    "mode": "selectable",
    "default": "scratch",
    "templates": {
      "scratch": {
        "template_revision_id": "sbxtplrev_...",
        "target": { "kind": "vfs", "provider": "bashkit" }
      },
      "build": {
        "template_revision_id": "sbxtplrev_...",
        "target": { "kind": "managed", "provider": "daytona" },
        "durability": "checkpointed"
      }
    }
  }
}
```

Select a declared binding in Playground when the mode is `selectable` or `configurable`:

```json
{
  "agent_id": "agent_...",
  "sandbox": { "use": "build" }
}
```

Use `GET /v1/sandbox-targets` to discover available targets,
`GET /v1/sessions/{session_id}/sandbox` to inspect the resolved Sandbox, and
`GET /v1/sandbox-templates` to list reusable templates.

## Resource Sandbox fleets

The primary Sandbox is implicit: ordinary shell and file tools always address it. An Agent with the
`sandbox_fleet` capability can additionally create explicitly addressed resource Sandboxes through
`sandbox_create`, then use `sandbox_exec`, `sandbox_read_file`, `sandbox_write_file`,
`sandbox_inspect`, `sandbox_checkpoint`, and `sandbox_manage`. Fleet operations never change where
ordinary shell or file tools run.
