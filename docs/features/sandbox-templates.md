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

With a managed provider, every file tool works on the Sandbox's `/workspace`: `read_file`,
`read_many_files`, `write_file`, `edit_file`, `glob`, `grep`, and `list_directory`. Tools that only
exist for the Session's own files, such as `grep_files` and `delete_file`, are not offered. Skills
under `/workspace/.agents/skills/` and saved tool output under `/workspace/outputs/` stay readable:
when the Sandbox does not hold such a file, the read falls back to the Session's files.

## Create a Sandbox Template

1. Open **Sandbox Templates** and select **New Sandbox Template**.
2. Give it a stable name and display name.
3. Select Bashkit or an available managed provider: Daytona, E2B, or Modal (when enabled).
4. For a managed provider, choose its **Provider account**: whose connection the Sandbox uses,
   and so whose provider account pays. Each choice is labelled with the provider, for example
   **Daytona · Personal**:
   - **Personal** uses the connection of the person starting the Session. A Session started by a
     trigger or a channel has no user, so it uses the organization's provider account instead,
     when the organization has exactly one for that provider. With none or several, the Session
     fails to start and names the problem; pick **Organization account** to choose one.
   - **This Agent** uses the Agent Virtual User's connection for every Session.
   - **Organization account** uses one shared account managed by an organization admin under
     **Sandboxes → Provider accounts**. When the organization has none for the provider, the
     picker links there to add one.
5. Configure durability, lifecycle, and bootstrap options, then save.

Editing a Sandbox Template creates a new immutable revision. Existing Sessions keep the revision
they already reference. Managed templates such as Bashkit Virtual Workspace are
provisioned by Everruns and cannot be edited or archived.

## Configure an Agent

1. Open **Agents**, then open the Agent.
2. Under **More**, select **Sandbox**.
3. Add a Sandbox Template. The Agent stores its exact revision, not a moving pointer.
4. Choose the policy mode:
   - **Fixed**: every Session gets the single configured Sandbox; Session overrides are rejected.
   - **Selectable**: Playground can select one of the Agent's declared template bindings.
   - **Configurable**: Playground may also submit constrained one-off configuration.
5. Choose the default binding when the policy permits multiple choices.
6. Select **Done**, then **Save changes**.

Bashkit is available without a provider connection. Managed providers are shown as unavailable
when their integration is not installed. User credentials live under **Settings → My agent
experience**, Agent credentials under the Agent's Virtual User, and organization credentials under
**Sandboxes → Provider Accounts**. A fixed Agent policy fixes this choice; Session creation cannot
override it. Do not put credentials in target options or bootstrap commands.

Use Sandbox Templates to select managed provider credentials. Raw `session_sandbox`
capability configurations cannot supply credential bindings or API/toolbox endpoint overrides;
the API rejects them on Agent, Harness, and Session writes.

The built-in [Bashkit Worker](/built-ins/harnesses/bashkit-worker/) seals the primary Sandbox to
Everruns' managed Bashkit template. Agents based on it cannot change the policy, and Session
creation cannot override it. Use [Sandbox Worker](/built-ins/harnesses/sandbox-worker/) when an
Agent must select Daytona, a container or another full sandbox; it requires one.

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

## Idle and retention

**When idle** and **Idle after** decide what happens to a running Sandbox nobody is using:
**Checkpoint and stop** or **Stop** pauses it, **Keep running** leaves it on, and **Delete**
deletes it. A paused Sandbox resumes on the next command.

The Sandbox at the provider is disposable: if the Session runs again after its Sandbox was
deleted, it gets a fresh one from the same template and runs the bootstrap commands again. Pick
**Delete** for work that starts from scratch each time, such as reviewing a pull request.

Paused Sandboxes still use provider storage, so Everruns deletes them after **Delete after
paused**: one day by default, or 3, 7, or 30 days, or **Never**. Deleting removes the Sandbox
and its files at the provider, not the Session. The Sandbox's record and history stay on the
**Sandboxes** page under **Deleted**.

```json
{
  "lifecycle": {
    "idle_after_seconds": 900,
    "idle_action": "checkpoint_and_stop",
    "delete_after_seconds": 604800
  }
}
```

Set `delete_after_seconds` to `null` to keep paused Sandboxes until the Session is deleted.

## Outbound network

Daytona, E2B and Modal templates can limit what the Sandbox reaches on the network. Under
**Outbound network**, choose **Open**, **Only listed domains** (one per line, `*.example.com` for
subdomains), or **Blocked**. The provider enforces the rule on its own network, so it holds for
every process in the Sandbox.

On Daytona and E2B the Session's network access list narrows the rule further: the Sandbox
reaches only hosts both allow, and a Sandbox resumed later follows the Session's current list.
Sandboxes the agent creates with the provider's own tools, desktops included, get the Session's
list. Providers filter by host, so a URL rule such as `https://pypi.org/simple/` opens all of
`pypi.org` inside the Sandbox.

```json
{
  "containment": {
    "level": "isolated",
    "network": { "mode": "allowlist", "allowed_hosts": ["github.com", "*.github.com"] }
  }
}
```

## Desktop

Daytona and E2B templates can start the primary Sandbox with a Linux desktop. Turn on **Desktop**
in the template, or set `target.options.desktop` to `true`. The Agent then gets the `computer`
tool: it takes screenshots of the desktop and clicks, types and scrolls on it. The desktop runs in
the same Sandbox as the shell and file tools, so files the agent writes are on the desktop too,
and the template's outbound network rule covers the desktop's browser.

To pick the display size or cap the number of actions, pass the computer-use settings instead of
`true`:

```json
{
  "target": {
    "kind": "managed",
    "provider": "e2b",
    "options": {
      "desktop": { "display_width": 1280, "display_height": 800, "max_actions_per_session": 300 }
    }
  }
}
```

The desktop is the Agent's only display: a template with a desktop replaces any other
computer-use capability the Agent has. On E2B the Sandbox starts from E2B's `desktop` template
unless the template names another one built on it. On Daytona it uses Daytona's default image,
which ships the desktop, so a desktop template cannot also choose a size or snapshot. Every action
returns a screenshot, which costs image tokens.

## Recovery after compute loss

A `checkpointed` Sandbox separates durable workspace state from replaceable physical compute.
Everruns checkpoints successful mutations for targets with `checkpointed` durability. If a Daytona
instance disappears, the next sandbox operation creates replacement compute, restores the workspace,
and continues the same Session. E2B and Modal use provider snapshots instead; their UI labels that
weaker recovery class explicitly.

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
    "target": {
      "kind": "managed",
      "provider": "daytona",
      "credential": { "source": "organization", "connection_id": "..." }
    },
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
