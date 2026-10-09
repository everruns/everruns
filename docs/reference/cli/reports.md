---
title: everruns reports
description: "Usage and cost reporting: queries, saved reports, catalog. CLI reference for everruns reports."
sidebar:
  label: reports
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Usage and cost reporting: queries, saved reports, catalog.

| Command | What it does |
|---|---|
| [`reports admin backfill`](#reports-admin-backfill) | Enqueue missing reporting projection work from canonical sources. |
| [`reports admin diagnostics get`](#reports-admin-diagnostics-get) | Inspect reporting projector lag and failed outbox rows. |
| [`reports catalog get`](#reports-catalog-get) | Return semantic reporting datasets, dimensions, measures, and filter fields. |
| [`reports projector run`](#reports-projector-run) | Claim and process pending reporting outbox rows. |
| [`reports query export`](#reports-query-export) | Run and export an org-scoped semantic reporting query. |
| [`reports query run`](#reports-query-run) | Run an org-scoped semantic reporting query. |
| [`reports saved create`](#reports-saved-create) | Create an org-scoped saved report definition. |
| [`reports saved delete`](#reports-saved-delete) | Delete an org-scoped saved report definition. |
| [`reports saved export`](#reports-saved-export) | Run and export an org-scoped saved report definition. |
| [`reports saved get`](#reports-saved-get) | Get an org-scoped saved report definition. |
| [`reports saved list`](#reports-saved-list) | List org-scoped saved report definitions. |
| [`reports saved run`](#reports-saved-run) | Run an org-scoped saved report definition. |
| [`reports saved update`](#reports-saved-update) | Update an org-scoped saved report definition. |

## reports admin backfill

Enqueue missing reporting projection work from canonical sources.

```bash
everruns reports admin backfill [OPTIONS]
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Maximum number of outbox rows to enqueue across all source types. |

Example:

```bash
# Rebuild reporting data that is missing from older sessions
everruns reports admin backfill --limit 1000 --reason 'Reports are missing last week'
```

## reports admin diagnostics get

Inspect reporting projector lag and failed outbox rows.

```bash
everruns reports admin diagnostics get [OPTIONS]
```

Example:

```bash
# Check whether reporting is lagging behind live data
everruns reports admin diagnostics get
```

## reports catalog get

Return semantic reporting datasets, dimensions, measures, and filter fields.

```bash
everruns reports catalog get [OPTIONS]
```

Example:

```bash
# Discover the datasets, dimensions and measures a query can use
everruns reports catalog get
```

## reports projector run

Claim and process pending reporting outbox rows.

```bash
everruns reports projector run [OPTIONS]
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Maximum number of items returned in this page. |

Example:

```bash
# Process pending reporting rows right away when reports look stale
everruns reports projector run --limit 500 --reason 'Reports lag behind live data'
```

## reports query export

Run and export an org-scoped semantic reporting query.

```bash
everruns reports query export [OPTIONS] --query <query>
```

| Flag | Description |
|---|---|
| `--format <FORMAT>` | Output format for a report export. One of `csv`, `json`. |
| `--query <QUERY>` | Required. Semantic query a caller submits to the reporting layer. |

Example:

```bash
# Download a one-off query result as CSV
everruns reports query export --format csv --query '{"dataset":"sessions","time_range":{"from":"2026-04-01T00:00:00Z","to":"2026-05-01T00:00:00Z"},"dimensions":["status"],"measures":["session_count"],"filters":[],"order_by":[],"limit":100}'
```

## reports query run

Run an org-scoped semantic reporting query.

```bash
everruns reports query run [OPTIONS] --dataset <dataset> --time-range <time_range>
```

| Flag | Description |
|---|---|
| `--dataset <DATASET>` | Required. Dataset name to query (see `GET /v1/reports/catalog` for the list of available datasets). |
| `--dimensions <DIMENSIONS>` | Columns to group by. Repeatable. |
| `--filters <FILTERS>` | Predicate filters applied before aggregation. |
| `--limit <LIMIT>` | Maximum number of rows to return (defaults to 100). |
| `--measures <MEASURES>` | Aggregations to compute (count, sum, avg, etc.). Repeatable. |
| `--order-by <ORDER_BY>` | Sort spec applied after aggregation. |
| `--time-range <TIME_RANGE>` | Required. Half-open time window applied to the dataset's primary timestamp column during a report query. |

Example:

```bash
# Answer a quick question, such as sessions per status last month
everruns reports query run --dataset sessions --time-range '{"from":"2026-04-01T00:00:00Z","to":"2026-05-01T00:00:00Z"}' --dimensions status --measures session_count
```

## reports saved create

Create an org-scoped saved report definition.

```bash
everruns reports saved create [OPTIONS] --name <name> --query <query>
```

| Flag | Description |
|---|---|
| `--dashboard <DASHBOARD>` |  |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--name <NAME>` | Required. Human-readable name. |
| `--query <QUERY>` | Required. Semantic query a caller submits to the reporting layer. |

Example:

```bash
# Save a query so the team can rerun it
everruns reports saved create --name 'Sessions by status' --query '{"dataset":"sessions","time_range":{"from":"2026-04-01T00:00:00Z","to":"2026-05-01T00:00:00Z"},"dimensions":["status"],"measures":["session_count"],"filters":[],"order_by":[],"limit":100}' --reason 'Monthly review'
```

## reports saved delete

Delete an org-scoped saved report definition.

```bash
everruns reports saved delete [OPTIONS] --report-id <report_id>
```

| Flag | Description |
|---|---|
| `--report-id <REPORT_ID>` | Required. Saved report's prefixed public identifier. |

Example:

```bash
# Delete a saved report nobody uses
everruns reports saved delete --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90 --reason 'Superseded by the weekly report'
```

## reports saved export

Run and export an org-scoped saved report definition.

```bash
everruns reports saved export [OPTIONS] --report-id <report_id> --request <request>
```

| Flag | Description |
|---|---|
| `--report-id <REPORT_ID>` | Required. Saved report's prefixed public identifier. |
| `--request <REQUEST>` | Required. Request body for the `export_saved_report` operation. |

Example:

```bash
# Download a saved report's current data as JSON
everruns reports saved export --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90 --request '{"format":"json"}'
```

## reports saved get

Get an org-scoped saved report definition.

```bash
everruns reports saved get [OPTIONS] --report-id <report_id>
```

| Flag | Description |
|---|---|
| `--report-id <REPORT_ID>` | Required. Saved report's prefixed public identifier. |

Example:

```bash
# Read a saved report's query before changing or running it
everruns reports saved get --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90
```

## reports saved list

List org-scoped saved report definitions.

```bash
everruns reports saved list [OPTIONS]
```

Example:

```bash
# Find a saved report's id
everruns reports saved list
```

## reports saved run

Run an org-scoped saved report definition.

```bash
everruns reports saved run [OPTIONS] --report-id <report_id>
```

| Flag | Description |
|---|---|
| `--report-id <REPORT_ID>` | Required. Saved report's prefixed public identifier. |

Example:

```bash
# Get the current numbers for a saved report
everruns reports saved run --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90
```

## reports saved update

Update an org-scoped saved report definition.

```bash
everruns reports saved update [OPTIONS] --report-id <report_id> --request <request>
```

| Flag | Description |
|---|---|
| `--report-id <REPORT_ID>` | Required. Saved report's prefixed public identifier. |
| `--request <REQUEST>` | Required. Request body for the `update_saved_report` operation. |

Example:

```bash
# Rename a saved report
everruns reports saved update --report-id 0190f8a2-7c1e-7d3a-9b2f-4e5d6c7b8a90 --request '{"name":"Weekly active agents"}' --reason 'Clearer name'
```
