---
title: everruns health-issues
description: "Operational problems detected in this installation. CLI reference for everruns health-issues."
sidebar:
  label: health-issues
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Operational problems detected in this installation.

| Command | What it does |
|---|---|
| [`health-issues check`](#health-issues-check) | Verify an installation's current health without modifying provider data. |
| [`health-issues get`](#health-issues-get) | Get an operational health issue. |
| [`health-issues list`](#health-issues-list) | List pending operational health issues. |
| [`health-issues snooze`](#health-issues-snooze) | Snooze health reminders for the current user for one day. |

## health-issues check

Verify an installation's current health without modifying provider data.

```bash
everruns health-issues check [OPTIONS] --issue-id <issue_id>
```

| Flag | Description |
|---|---|
| `--issue-id <ISSUE_ID>` | Required. Stable identifier of the issue in the current organization. |

Example:

```bash
# Recheck whether an issue is resolved after fixing it
everruns health-issues check --issue-id 550e8400-e29b-41d4-a716-446655440000
```

## health-issues get

Get an operational health issue.

```bash
everruns health-issues get [OPTIONS] --issue-id <issue_id>
```

| Flag | Description |
|---|---|
| `--issue-id <ISSUE_ID>` | Required. Stable identifier of the issue in the current organization. |

Example:

```bash
# Read an issue's evidence and recovery steps
everruns health-issues get --issue-id 550e8400-e29b-41d4-a716-446655440000
```

## health-issues list

List pending operational health issues.

```bash
everruns health-issues list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--channel-id <CHANNEL_ID>` | Restrict results to this public channel identifier. |
| `--limit <LIMIT>` | Page size, clamped to 1 through 100; defaults to 20. |
| `--offset <OFFSET>` | Number of matching issues to skip; defaults to zero. |

Example:

```bash
# See what operational problems need attention
everruns health-issues list --limit 10
```

## health-issues snooze

Snooze health reminders for the current user for one day.

```bash
everruns health-issues snooze [OPTIONS] --issue-id <issue_id>
```

| Flag | Description |
|---|---|
| `--issue-id <ISSUE_ID>` | Required. Stable identifier of the issue in the current organization. |

Example:

```bash
# Silence your reminders for an issue for a day
everruns health-issues snooze --issue-id 550e8400-e29b-41d4-a716-446655440000 --reason 'Fix scheduled for tomorrow'
```
