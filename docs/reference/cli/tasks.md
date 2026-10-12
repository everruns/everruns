---
title: everruns tasks
description: "Background tasks across every session in the organization. CLI reference for everruns tasks."
sidebar:
  label: tasks
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Background tasks across every session in the organization.

| Command | What it does |
|---|---|
| [`tasks list`](#tasks-list) | List background tasks across every session in the org. |

## tasks list

List background tasks across every session in the org.

```bash
everruns tasks list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--created-after <CREATED_AFTER>` | Optional age filter: only tasks created at or after this RFC3339 timestamp. |
| `--kind <KIND>` | Optional kind filter (subagent, external_agent, background_tool, monitor, ...). |
| `--limit <LIMIT>` | Max tasks to return, newest first. |
| `--root-session-id <ROOT_SESSION_ID>` | Optional delegation-tree filter (EVE-680): only tasks whose owning session's root is this ses... |
| `--state <STATE>` | Optional state filter (queued, running, awaiting_input, succeeded, failed, canceled). |

Example:

```bash
# Find failed background work anywhere in the org
everruns tasks list --state failed --limit 20
```
