---
title: everruns durable
description: "Durable scheduled tasks and their executions. CLI reference for everruns durable."
sidebar:
  label: durable
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Durable scheduled tasks and their executions.

| Command | What it does |
|---|---|
| [`durable executions get`](#durable-executions-get) | Get a durable schedule execution. |
| [`durable schedules create`](#durable-schedules-create) | Create a new durable scheduled task with a cron expression. |
| [`durable schedules delete`](#durable-schedules-delete) | Delete a durable schedule. |
| [`durable schedules get`](#durable-schedules-get) | Get a durable schedule. |
| [`durable schedules list`](#durable-schedules-list) | List durable scheduled tasks. |
| [`durable schedules pause`](#durable-schedules-pause) | Pause a durable schedule. |
| [`durable schedules resume`](#durable-schedules-resume) | Resume a durable schedule. |
| [`durable schedules trigger`](#durable-schedules-trigger) | Manually trigger a durable schedule. |
| [`durable schedules update`](#durable-schedules-update) | Update a durable schedule. |
| [`durable schedules executions list`](#durable-schedules-executions-list) | List executions for a durable schedule. |
| [`durable schedules stats get`](#durable-schedules-stats-get) | Get durable schedule statistics. |

## durable executions get

Get a durable schedule execution.

```bash
everruns durable executions get [OPTIONS] [EXECUTION_ID]
```

| Flag | Description |
|---|---|
| `--execution-id <EXECUTION_ID>` | Schedule execution's identifier. |

Example:

```bash
# Inspect one run of a schedule, including its error if it failed
everruns durable executions get exec_01h9
```

## durable schedules create

Create a new durable scheduled task with a cron expression.

```bash
everruns durable schedules create [OPTIONS] --cron-expression <cron_expression> --name <name> --target <target>
```

| Flag | Description |
|---|---|
| `--catch-up-missed` | Whether to catch up missed triggers (default: false) |
| `--cron-expression <CRON_EXPRESSION>` | Required. Cron expression (5-field or 7-field). |
| `--description <DESCRIPTION>` | Optional description. |
| `--enabled` | Whether schedule is enabled (default: true) |
| `--max-catch-up <MAX_CATCH_UP>` | Max catch-up executions when `catch_up_missed` is true. |
| `--max-concurrent <MAX_CONCURRENT>` | Max concurrent executions. |
| `--name <NAME>` | Required. Unique name for the schedule. |
| `--retry-policy <RETRY_POLICY>` | Retry policy for failed executions (provider-specific JSON; see the durable engine's `RetryPo... |
| `--target <TARGET>` | Required. Target for a schedule - either a workflow or activity. |
| `--timezone <TIMEZONE>` | Timezone (default: UTC). |

Example:

```bash
# Run a session workflow every night
everruns durable schedules create --name nightly-triage --cron-expression '0 2 * * *' --timezone UTC --target '{"type":"workflow","name":"session.run","input":{"session_id":"session_01h9"}}' --reason 'Nightly triage'
```

## durable schedules delete

Delete a durable schedule.

```bash
everruns durable schedules delete [OPTIONS] --schedule-id <schedule_id>
```

| Flag | Description |
|---|---|
| `--schedule-id <SCHEDULE_ID>` | Required. Schedule's prefixed public identifier. |

Example:

```bash
# Remove a schedule you no longer need
everruns durable schedules delete --schedule-id sched_01h9 --reason 'Replaced by a webhook trigger'
```

## durable schedules get

Get a durable schedule.

```bash
everruns durable schedules get [OPTIONS] [SCHEDULE_ID]
```

| Flag | Description |
|---|---|
| `--schedule-id <SCHEDULE_ID>` | Schedule's prefixed public identifier. |

Example:

```bash
# Check a schedule's cron expression, target and next run
everruns durable schedules get sched_01h9
```

## durable schedules list

List durable scheduled tasks.

```bash
everruns durable schedules list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--enabled` | Filter by enabled status. |
| `--limit <LIMIT>` | Pagination limit (default: 20, max: 100) |
| `--offset <OFFSET>` | Pagination offset. |
| `--target-type <TARGET_TYPE>` | Filter by target type ("workflow" or "activity") |

Example:

```bash
# Find schedules that are still enabled
everruns durable schedules list --enabled true
```

## durable schedules pause

Pause a durable schedule.

```bash
everruns durable schedules pause [OPTIONS] --schedule-id <schedule_id>
```

| Flag | Description |
|---|---|
| `--schedule-id <SCHEDULE_ID>` | Required. Schedule's prefixed public identifier. |

Example:

```bash
# Stop a schedule from firing while you investigate
everruns durable schedules pause --schedule-id sched_01h9 --reason 'Investigate repeated failures'
```

## durable schedules resume

Resume a durable schedule.

```bash
everruns durable schedules resume [OPTIONS] --schedule-id <schedule_id>
```

| Flag | Description |
|---|---|
| `--schedule-id <SCHEDULE_ID>` | Required. Schedule's prefixed public identifier. |

Example:

```bash
# Turn a paused schedule back on
everruns durable schedules resume --schedule-id sched_01h9 --reason 'Failures fixed'
```

## durable schedules trigger

Manually trigger a durable schedule.

```bash
everruns durable schedules trigger [OPTIONS] --schedule-id <schedule_id>
```

| Flag | Description |
|---|---|
| `--schedule-id <SCHEDULE_ID>` | Required. Schedule's prefixed public identifier. |

Example:

```bash
# Run a schedule once now to test its target
everruns durable schedules trigger --schedule-id sched_01h9 --reason 'Verify the target before the next run'
```

## durable schedules update

Update a durable schedule.

```bash
everruns durable schedules update [OPTIONS] --schedule-id <schedule_id>
```

| Flag | Description |
|---|---|
| `--catch-up-missed` | Catch up missed triggers. |
| `--cron-expression <CRON_EXPRESSION>` | New cron expression. |
| `--description <DESCRIPTION>` | New description. |
| `--enabled` | Enable/disable. |
| `--max-catch-up <MAX_CATCH_UP>` | Max catch-up executions. |
| `--max-concurrent <MAX_CONCURRENT>` | Max concurrent executions. |
| `--retry-policy <RETRY_POLICY>` | Retry policy (provider-specific JSON; see the durable engine's `RetryPolicy`). Example: `{"ma... |
| `--schedule-id <SCHEDULE_ID>` | Required. Schedule's prefixed public identifier. |
| `--target <TARGET>` |  |
| `--timezone <TIMEZONE>` | New timezone (IANA name). |

Example:

```bash
# Move a schedule to a different time
everruns durable schedules update --schedule-id sched_01h9 --cron-expression '0 3 * * *' --reason 'Avoid the 02:00 backup window'
```

## durable schedules executions list

List executions for a durable schedule.

```bash
everruns durable schedules executions list [OPTIONS] --schedule-id <schedule_id>
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Pagination limit (default: 20, max: 100) |
| `--offset <OFFSET>` | Pagination offset. |
| `--schedule-id <SCHEDULE_ID>` | Required. Schedule ID. |
| `--status <STATUS>` | Filter by execution status. |

Example:

```bash
# Find recent failed runs of a schedule
everruns durable schedules executions list --schedule-id sched_01h9 --status failed
```

## durable schedules stats get

Get durable schedule statistics.

```bash
everruns durable schedules stats get [OPTIONS] --schedule-id <schedule_id>
```

| Flag | Description |
|---|---|
| `--schedule-id <SCHEDULE_ID>` | Required. Schedule's prefixed public identifier. |

Example:

```bash
# See a schedule's success and failure counts
everruns durable schedules stats get --schedule-id sched_01h9
```
