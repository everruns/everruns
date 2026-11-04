---
title: everruns system
description: "Server status. CLI reference for everruns system: usage, flags and examples."
sidebar:
  label: system
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Server status.

| Command | What it does |
|---|---|
| [`system health`](#system-health) | Health check endpoint. |
| [`system detections`](#system-detections) | List open abuse detections in every organization, with the Inspector's write-up. |
| [`system doctor`](#system-doctor) | Check whether the whole install is set up right and working: database, workers, keys, messaging, storage, monitoring. |

## system health

Health check endpoint.

```bash
everruns system health [OPTIONS]
```

Example:

```bash
# Check that the control plane is reachable and healthy
everruns system health
```

## system detections

List open abuse detections in every organization, with the Inspector's write-up. Leads for a person to confirm, never grounds for automatic action.

```bash
everruns system detections [OPTIONS]
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Page size from 1 through 100; defaults to 20. |
| `--offset <OFFSET>` | Number of detections to skip; defaults to zero. |

Example:

```bash
# Review sessions suspected of abusing the platform
everruns system detections --limit 20
```

## system doctor

Check whether the whole install is set up right and working: database, workers, keys, messaging, storage, monitoring.

```bash
everruns system doctor [OPTIONS]
```

| Flag | Description |
|---|---|
| `--refresh` | Run every check now instead of returning the report from the last minute. |

Example:

```bash
# Check every part of the install and list what needs fixing
everruns system doctor --refresh true
```
