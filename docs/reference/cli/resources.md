---
title: everruns resources
description: "Resources commands. CLI reference for everruns resources: usage, flags and examples."
sidebar:
  label: resources
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Resources commands.

| Command | What it does |
|---|---|
| [`resources grants grant`](#resources-grants-grant) | Explicitly grant bounded resource use to a consumer. |
| [`resources grants list`](#resources-grants-list) | List explicit use grants for an authorized resource owner. |
| [`resources grants revoke`](#resources-grants-revoke) | Revoke an explicit resource use grant. |

## resources grants grant

Explicitly grant bounded resource use to a consumer. Use does not grant management or resharing.

```bash
everruns resources grants grant [OPTIONS] --consumer-scope <consumer_scope> --id <id> --usage <usage>
```

| Flag | Description |
|---|---|
| `--constraints <CONSTRAINTS>` |  |
| `--consumer-id <CONSUMER_ID>` |  |
| `--consumer-scope <CONSUMER_SCOPE>` | Required. One of `session`, `agent`, `virtual_user`, `organization`. |
| `--expires-at <EXPIRES_AT>` |  |
| `--id <ID>` | Required. |
| `--usage <USAGE>` | Required. One of `user`, `service`. |

Example:

```bash
# Allow one service agent to use an organization credential
everruns resources grants grant --id 00000000-0000-0000-0000-000000000001 --consumer-scope agent --consumer-id agent_00000000000000000000000000000002 --usage service
```

## resources grants list

List explicit use grants for an authorized resource owner.

```bash
everruns resources grants list [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |

Example:

```bash
# Review who can use a credential or connection
everruns resources grants list --id 00000000-0000-0000-0000-000000000001
```

## resources grants revoke

Revoke an explicit resource use grant.

```bash
everruns resources grants revoke [OPTIONS] --grant-id <grant_id> --id <id>
```

| Flag | Description |
|---|---|
| `--grant-id <GRANT_ID>` | Required. |
| `--id <ID>` | Required. |

Example:

```bash
# Stop a consumer from using a resource without revoking its other grants
everruns resources grants revoke --id 00000000-0000-0000-0000-000000000001 --grant-id 00000000-0000-0000-0000-000000000003
```
