---
title: everruns system
description: "Server status. CLI reference for everruns system."
sidebar:
  label: system
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Server status.

| Command | What it does |
|---|---|
| [`system health`](#system-health) | Health check endpoint. |

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
