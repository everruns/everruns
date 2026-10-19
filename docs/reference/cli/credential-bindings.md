---
title: everruns credential-bindings
description: "Declare, list and revoke write-only credential bindings. CLI reference for everruns credential-bindings: usage, flags and examples."
sidebar:
  label: credential-bindings
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Declare, list and revoke write-only credential bindings.

| Command | What it does |
|---|---|
| [`credential-bindings create`](#credential-bindings-create) | Declare an HTTP or MCP write-only credential binding. |
| [`credential-bindings list`](#credential-bindings-list) | List credential metadata for an authorized owner. |
| [`credential-bindings revoke`](#credential-bindings-revoke) | Revoke a credential at the observed revision; future sends fail closed. |

## credential-bindings create

Declare an HTTP or MCP write-only credential binding. The owner provisions its value in the secure Credentials UI.

```bash
everruns credential-bindings create [OPTIONS] --label <label> --scope <scope> --usage <usage>
```

| Flag | Description |
|---|---|
| `--binding <BINDING>` |  |
| `--label <LABEL>` | Required. |
| `--mcp <MCP>` |  |
| `--name <NAME>` |  |
| `--owner-id <OWNER_ID>` |  |
| `--scope <SCOPE>` | Required. One of `session`, `agent`, `virtual_user`, `organization`. |
| `--source <SOURCE>` |  |
| `--usage <USAGE>` | Required. One of `user`, `service`. |

Example:

```bash
# Declare personal mailbox access without putting a value in the command
everruns credential-bindings create --scope virtual_user --owner-id me --usage user --name MAIL_TOKEN --label 'Mailbox API' --binding '{"url":"https://mail.example/api","auth":{"type":"bearer"}}'
```

## credential-bindings list

List credential metadata for an authorized owner. Values are never returned.

```bash
everruns credential-bindings list [OPTIONS] --scope <scope> --usage <usage>
```

| Flag | Description |
|---|---|
| `--owner-id <OWNER_ID>` |  |
| `--scope <SCOPE>` | Required. One of `session`, `agent`, `virtual_user`, `organization`. |
| `--usage <USAGE>` | Required. One of `user`, `service`. |

Example:

```bash
# Review organization credential metadata before granting access
everruns credential-bindings list --scope organization --usage service
```

## credential-bindings revoke

Revoke a credential at the observed revision; future sends fail closed.

```bash
everruns credential-bindings revoke [OPTIONS] --id <id> --revision <revision>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |
| `--revision <REVISION>` | Required. |

Example:

```bash
# Revoke a credential using the revision returned by the metadata list
everruns credential-bindings revoke --id 00000000-0000-0000-0000-000000000001 --revision 3
```
