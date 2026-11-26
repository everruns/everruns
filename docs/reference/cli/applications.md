---
title: everruns applications
description: "Published workspaces with Chats, Rooms, shared Memory and one audience. CLI reference for everruns applications: usage, flags and examples."
sidebar:
  label: applications
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Published workspaces with Chats, Rooms, shared Memory and one audience.

| Command | What it does |
|---|---|
| [`applications create`](#applications-create) | Create an Application with a permanently reserved stable name. |
| [`applications get`](#applications-get) | Get an Application's configuration and resources. |
| [`applications list`](#applications-list) | List Applications in the current organization. |
| [`applications status`](#applications-status) | Publish, unpublish or archive an Application. |
| [`applications update`](#applications-update) | Update an Application and its resource bindings, checking its version. |
| [`applications resources create`](#applications-resources-create) | Create and attach a Chat, Room or Memory atomically to an Application. |

## applications create

Create an Application with a permanently reserved stable name.

```bash
everruns applications create [OPTIONS] --name <name> --slug <slug>
```

| Flag | Description |
|---|---|
| `--config <CONFIG>` | Audience admission and optional workspace features. |
| `--description <DESCRIPTION>` | Overview text for participants, up to 2000 characters. |
| `--name <NAME>` | Required. Display name, between 1 and 120 characters after trimming. |
| `--resources <RESOURCES>` | Ordered resource bindings; at most 50, with unique paths and IDs. |
| `--slug <SLUG>` | Required. Permanent hostname label, globally unique and reserved after archival. |

Example:

```bash
# Reserve a workspace name for organization users
everruns applications create --name 'Content Studio' --slug content-studio --config '{"membership":"org_users"}' --reason 'Create the editorial workspace'
```

## applications get

Get an Application's configuration and resources.

```bash
everruns applications get [OPTIONS] --application-id <application_id>
```

| Flag | Description |
|---|---|
| `--application-id <APPLICATION_ID>` | Required. Prefixed identifier with 'application' prefix. |

Example:

```bash
# Read the current resource bindings and version
everruns applications get --application-id application_01a213c3f773718e9d2b8c895eee5d36
```

## applications list

List Applications in the current organization.

```bash
everruns applications list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived Applications so their owners can restore them as drafts. |

Example:

```bash
# Find active and archived workspaces
everruns applications list --include-archived
```

## applications status

Publish, unpublish or archive an Application. Archiving retains its stable name.

```bash
everruns applications status [OPTIONS] --application-id <application_id> --status <status> --version <version>
```

| Flag | Description |
|---|---|
| `--application-id <APPLICATION_ID>` | Required. Prefixed identifier with 'application' prefix. |
| `--status <STATUS>` | Required. One of `draft`, `published`, `archived`. |
| `--version <VERSION>` | Required. Current version from the latest read. |

Example:

```bash
# Publish the saved workspace and its audience
everruns applications status --application-id application_01a213c3f773718e9d2b8c895eee5d36 --version 3 --status published --reason 'Open the workspace to its audience'
```

## applications update

Update an Application and its resource bindings, checking its version.

```bash
everruns applications update [OPTIONS] --application-id <application_id> --name <name> --slug <slug> --version <version>
```

| Flag | Description |
|---|---|
| `--application-id <APPLICATION_ID>` | Required. Prefixed identifier with 'application' prefix. |
| `--config <CONFIG>` | Audience admission and optional workspace features. |
| `--description <DESCRIPTION>` | Overview text for participants, up to 2000 characters. |
| `--name <NAME>` | Required. Display name, between 1 and 120 characters after trimming. |
| `--resources <RESOURCES>` | Ordered resource bindings; at most 50, with unique paths and IDs. |
| `--slug <SLUG>` | Required. Permanent hostname label, globally unique and reserved after archival. |
| `--version <VERSION>` | Required. Current version from the latest read; stale edits are rejected. |

Example:

```bash
# Rename an empty draft before attaching resources
everruns applications update --application-id application_01a213c3f773718e9d2b8c895eee5d36 --version 1 --name 'Editorial Studio' --slug content-studio --reason 'Name the new draft'
```

## applications resources create

Create and attach a Chat, Room or Memory atomically to an Application.

```bash
everruns applications resources create [OPTIONS] --application-id <application_id> --kind <kind> --name <name> --slug <slug> --version <version>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Existing Agent to use for a new Chat; required only for Chat resources. |
| `--application-id <APPLICATION_ID>` | Required. Prefixed identifier with 'application' prefix. |
| `--kind <KIND>` | Required. Existing runtime resource exposed through the Application. One of `chat`, `room`, `memory`. |
| `--name <NAME>` | Required. Navigation label and initial name for a new Room or Memory. |
| `--slug <SLUG>` | Required. Unique navigation path within the Application. |
| `--version <VERSION>` | Required. Current Application version; creation and binding are one transaction. |

Example:

```bash
# Create a Chat using an existing Agent
everruns applications resources create --application-id application_01a213c3f773718e9d2b8c895eee5d36 --version 1 --kind chat --name Writer --slug writer --agent-id agent_01a213c7b3cf76de8ec474d1c5b7fb27 --reason 'Add the writing assistant'
```
