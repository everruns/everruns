---
title: everruns sandboxes
description: "Sandboxes across providers: state, history, usage. CLI reference for everruns sandboxes."
sidebar:
  label: sandboxes
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Sandboxes across providers: state, history, usage.

| Command | What it does |
|---|---|
| [`sandboxes get`](#sandboxes-get) | Get one Sandbox with its provider resources and full state history. |
| [`sandboxes list`](#sandboxes-list) | List the organization's Sandboxes across providers: running, paused, lost and deleted, with filters. |
| [`sandboxes stats get`](#sandboxes-stats-get) | Summarize the organization's Sandboxes: counts by state and provider, recent creations, running time, recoveries and how many need attention. |
| [`sandboxes timeline get`](#sandboxes-timeline-get) | Show when each Sandbox was running, paused or lost over a time window, with how many ran at once. |

## sandboxes get

Get one Sandbox with its provider resources and full state history.

```bash
everruns sandboxes get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Logical Sandbox id. |

Example:

```bash
# See why a Sandbox was rebuilt
everruns sandboxes get sandbox_01h9
```

## sandboxes list

List the organization's Sandboxes across providers: running, paused, lost and deleted, with filters.

```bash
everruns sandboxes list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Only Sandboxes of Sessions that ran this Agent. |
| `--include-in-process` | Include in-process targets (virtual filesystem, host), which have no provider resource. |
| `--limit <LIMIT>` | Page size, 1 to 200. |
| `--needs-attention` | Only Sandboxes with at least one attention reason. |
| `--offset <OFFSET>` | Rows to skip. |
| `--provider <PROVIDER>` | Comma-separated provider ids, such as `daytona,modal`. |
| `--sandbox-template-id <SANDBOX_TEMPLATE_ID>` | Only Sandboxes created from this Sandbox Template. |
| `--search <SEARCH>` | Case-insensitive match on Session title, Agent, provider or provider resource id. |
| `--state <STATE>` | Comma-separated states: `running`, `paused`, `lost`, `starting`, `failed`, `not_started`, `de... |

Example:

```bash
# Find Sandboxes that need attention
everruns sandboxes list --needs-attention true
```

## sandboxes stats get

Summarize the organization's Sandboxes: counts by state and provider, recent creations, running time, recoveries and how many need attention.

```bash
everruns sandboxes stats get [OPTIONS]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Only Sandboxes of Sessions that ran this Agent. |
| `--include-in-process` | Include in-process targets (virtual filesystem, host), which have no provider resource. |
| `--needs-attention` | Only Sandboxes with at least one attention reason. |
| `--provider <PROVIDER>` | Comma-separated provider ids, such as `daytona,modal`. |
| `--sandbox-template-id <SANDBOX_TEMPLATE_ID>` | Only Sandboxes created from this Sandbox Template. |
| `--search <SEARCH>` | Case-insensitive match on Session title, Agent, provider or provider resource id. |
| `--state <STATE>` | Comma-separated states: `running`, `paused`, `lost`, `starting`, `failed`, `not_started`, `de... |

Example:

```bash
# See how many Sandboxes are running and how many need attention
everruns sandboxes stats get --provider daytona
```

## sandboxes timeline get

Show when each Sandbox was running, paused or lost over a time window, with how many ran at once.

```bash
everruns sandboxes timeline get [OPTIONS]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Only Sandboxes of Sessions that ran this Agent. |
| `--from <FROM>` | Window start (RFC 3339). |
| `--include-in-process` | Include in-process targets (virtual filesystem, host), which have no provider resource. |
| `--limit <LIMIT>` | Lanes to return, 1 to 200. |
| `--needs-attention` | Only Sandboxes with at least one attention reason. |
| `--provider <PROVIDER>` | Comma-separated provider ids, such as `daytona,modal`. |
| `--sandbox-template-id <SANDBOX_TEMPLATE_ID>` | Only Sandboxes created from this Sandbox Template. |
| `--search <SEARCH>` | Case-insensitive match on Session title, Agent, provider or provider resource id. |
| `--state <STATE>` | Comma-separated states: `running`, `paused`, `lost`, `starting`, `failed`, `not_started`, `de... |
| `--to <TO>` | Window end (RFC 3339). |

Example:

```bash
# See when Sandboxes ran or were lost over the last day
everruns sandboxes timeline get --from 2026-05-01T00:00:00Z --to 2026-05-02T00:00:00Z --state running,lost
```
