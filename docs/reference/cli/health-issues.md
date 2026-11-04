---
title: everruns health-issues
description: "Operational problems detected in this installation. CLI reference for everruns health-issues: usage, flags and examples."
sidebar:
  label: health-issues
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Operational problems detected in this installation.

| Command | What it does |
|---|---|
| [`health-issues check`](#health-issues-check) | Verify an installation's current health without modifying provider data. |
| [`health-issues dismiss`](#health-issues-dismiss) | Dismiss a detector finding with a reason. |
| [`health-issues get`](#health-issues-get) | Get an operational health issue. |
| [`health-issues list`](#health-issues-list) | List pending operational health issues. |
| [`health-issues fix`](#health-issues-fix) | Run a fixer the issue offers, such as adding missing Slack permissions to the agent's app. |
| [`health-issues snooze`](#health-issues-snooze) | Snooze health reminders for the current user for one day. |
| [`health-issues actions list`](#health-issues-actions-list) | List what Everruns or a person did about a finding, such as pausing a repeating schedule, and what the organization's self-healing policy offers to do now. |
| [`health-issues actions take`](#health-issues-actions-take) | Take an action the organization's self-healing policy offers on a finding, such as pausing a schedule whose runs keep ending the same way. |
| [`health-issues actions undo`](#health-issues-actions-undo) | Undo an action taken on a finding, automatic or not. |

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

## health-issues dismiss

Dismiss a detector finding with a reason.

```bash
everruns health-issues dismiss [OPTIONS] --issue-id <issue_id> --note <note>
```

| Flag | Description |
|---|---|
| `--issue-id <ISSUE_ID>` | Required. Stable identifier of the issue in the current organization. |
| `--note <NOTE>` | Required. A false alarm, expected work, or already handled. |

Example:

```bash
# Close a false alarm and say why
everruns health-issues dismiss --issue-id 550e8400-e29b-41d4-a716-446655440000 --note 'Expected: nightly load test'
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

## health-issues fix

Run a fixer the issue offers, such as adding missing Slack permissions to the agent's app. A fix that needs approval in the provider returns the page to approve it on; the issue resolves only when a fresh check confirms it.

```bash
everruns health-issues fix [OPTIONS] --issue-id <issue_id> --kind <kind>
```

| Flag | Description |
|---|---|
| `--issue-id <ISSUE_ID>` | Required. The issue. |
| `--kind <KIND>` | Required. The fixer to run, as the issue's `fixers` list names it. |

Example:

```bash
# Add missing Slack permissions to an agent's app
everruns health-issues fix --issue-id 550e8400-e29b-41d4-a716-446655440000 --kind slack.add_permissions
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

## health-issues actions list

List what Everruns or a person did about a finding, such as pausing a repeating schedule, and what the organization's self-healing policy offers to do now.

```bash
everruns health-issues actions list [OPTIONS] --issue-id <issue_id>
```

| Flag | Description |
|---|---|
| `--issue-id <ISSUE_ID>` | Required. The finding. |

Example:

```bash
# See whether a repeating schedule was paused
everruns health-issues actions list --issue-id 550e8400-e29b-41d4-a716-446655440000
```

## health-issues actions take

Take an action the organization's self-healing policy offers on a finding, such as pausing a schedule whose runs keep ending the same way. It is recorded on the finding and can be undone.

```bash
everruns health-issues actions take [OPTIONS] --issue-id <issue_id> --kind <kind>
```

| Flag | Description |
|---|---|
| `--issue-id <ISSUE_ID>` | Required. The finding. |
| `--kind <KIND>` | Required. `pause_schedule`. |

Example:

```bash
# Pause a repeating schedule from its finding
everruns health-issues actions take --issue-id 550e8400-e29b-41d4-a716-446655440000 --kind pause_schedule --reason 'Claim expired, polling is pointless'
```

## health-issues actions undo

Undo an action taken on a finding, automatic or not. Undoing a pause resumes the schedule and starts its repeated-outcome count over.

```bash
everruns health-issues actions undo [OPTIONS] --action-id <action_id> --issue-id <issue_id>
```

| Flag | Description |
|---|---|
| `--action-id <ACTION_ID>` | Required. Prefixed public identifier of the action (`fact_…`). |
| `--issue-id <ISSUE_ID>` | Required. The finding. |

Example:

```bash
# Resume a schedule that was paused automatically
everruns health-issues actions undo --issue-id 550e8400-e29b-41d4-a716-446655440000 --action-id fact_01h9 --reason 'Still waiting on the claim; this is expected'
```
