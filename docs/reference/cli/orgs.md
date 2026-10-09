---
title: everruns orgs
description: "Organizations you belong to, and which one is active. CLI reference for everruns orgs."
sidebar:
  label: orgs
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Organizations you belong to, and which one is active.

| Command | What it does |
|---|---|
| [`orgs select`](#orgs-select) | Interactive organization picker. |
| [`orgs get`](#orgs-get) | Get organization details. |
| [`orgs list`](#orgs-list) | List organizations for the current user. |
| [`orgs resolve`](#orgs-resolve) | Resolve the owning organization for a prefixed entity id (agent, session, harness, app, skill, mcp server, identity, eval). |
| [`orgs audit-logs list`](#orgs-audit-logs-list) | List audit logs for the caller's organization. |

## orgs select

Interactive organization picker.

```bash
everruns orgs select
```


## orgs get

Get organization details.

```bash
everruns orgs get [OPTIONS] [ORG]
```

| Flag | Description |
|---|---|
| `--org <ORG>` | Organization's prefixed public identifier. |

Example:

```bash
# Show an organization's details
everruns orgs get org_01h9
```

## orgs list

List organizations for the current user.

```bash
everruns orgs list [OPTIONS]
```

Example:

```bash
# See which organizations you belong to and their ids
everruns orgs list
```

## orgs resolve

Resolve the owning organization for a prefixed entity id (agent, session, harness, app, skill, mcp server, identity, eval). Requires an authenticated user; returns NotFound when the caller is not a member of the owning org or the id does not resolve.

```bash
everruns orgs resolve [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed public identifier. |

Example:

```bash
# Look up an organization by its identifier
everruns orgs resolve --id org_01h9
```

## orgs audit-logs list

List audit logs for the caller's organization. Supports domain, action, actor, and event-type filters.

```bash
everruns orgs audit-logs list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--action <ACTION>` | Filter by action string (e.g. |
| `--actor-id <ACTOR_ID>` | Filter by actor UUID. |
| `--before <BEFORE>` | Cursor: return entries created before this timestamp. |
| `--domain <DOMAIN>` | Filter by audit domain ("management" or "agent"). |
| `--event-type <EVENT_TYPE>` | Filter by event type prefix (e.g. |
| `--limit <LIMIT>` | Max entries to return (default 50, max 200). |

Example:

```bash
# Review who changed what in the organization recently
everruns orgs audit-logs list --domain management --limit 50
```
