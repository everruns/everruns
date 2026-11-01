---
title: everruns orgs
description: "Organizations you belong to, and which one is active. CLI reference for everruns orgs: usage, flags and examples."
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
| [`orgs update`](#orgs-update) | Update the active organization's name and defaults. |
| [`orgs audit-logs list`](#orgs-audit-logs-list) | List audit logs for the caller's organization. |
| [`orgs decision-settings get`](#orgs-decision-settings-get) | Read the active organization's decision-model selection without model credentials. |
| [`orgs egress-allowlist get`](#orgs-egress-allowlist-get) | Get the organization's outbound allowlist extension: whether a platform administrator granted it, its host patterns, and the deployment's egress policy mode. |
| [`orgs egress-allowlist set`](#orgs-egress-allowlist-set) | Replace the organization's outbound allowlist extension. |
| [`orgs egress-allowlist grant set`](#orgs-egress-allowlist-grant-set) | Platform administrators only: allow or stop an organization extending the outbound allowlist (revoking keeps its patterns but stops enforcing them), and give or take back open egress (any public host the deny list allows, reads and writes). |
| [`orgs provider-accounts list`](#orgs-provider-accounts-list) | List the organization's provider accounts (sandbox provider accounts such as Daytona or E2B, and web search accounts): id, name, provider, provider username, and timestamps. |

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

## orgs update

Update the active organization's name and defaults.

```bash
everruns orgs update [OPTIONS] --org <org>
```

| Flag | Description |
|---|---|
| `--agentid-agents-per-owner <AGENTID_AGENTS_PER_OWNER>` | How many agents one AgentID owner may sign in to this organization's Public Chat channels. |
| `--background-limits <BACKGROUND_LIMITS>` |  |
| `--base-harness-id <BASE_HARNESS_ID>` | Base harness to use when a session is started without an explicit harness_id. |
| `--chat-model-id <CHAT_MODEL_ID>` | Independent model for the built-in Everruns Chat. |
| `--default-harness-id <DEFAULT_HARNESS_ID>` | Default harness to preselect in the UI for new sessions. Mutually exclusive with `default_har... |
| `--default-harness-name <DEFAULT_HARNESS_NAME>` | Alternative to `default_harness_id` — looked up by stable name within the org. Mutually exclu... |
| `--default-model-id <DEFAULT_MODEL_ID>` | Default LLM model for this organization. |
| `--default-provider-per-service <DEFAULT_PROVIDER_PER_SERVICE>` | Org-level default provider per service (EVE-569). |
| `--name <NAME>` | The display name of the organization. |
| `--org <ORG>` | Required. |
| `--small-model-id <SMALL_MODEL_ID>` | Shared small chat model for lightweight work. |
| `--system-decisions <SYSTEM_DECISIONS>` |  |
| `--web-search-defaults <WEB_SEARCH_DEFAULTS>` |  |

Example:

```bash
# Rename the active organization while preserving its model and harness defaults
everruns orgs update --org org_01h9 --name 'Northstar Support' --reason 'Use the team name'
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

## orgs decision-settings get

Read the active organization's decision-model selection without model credentials.

```bash
everruns orgs decision-settings get [OPTIONS] --org <org>
```

| Flag | Description |
|---|---|
| `--org <ORG>` | Required. |

Example:

```bash
# Check the active organization decision-model selection
everruns orgs decision-settings get --org org_01h9
```

## orgs egress-allowlist get

Get the organization's outbound allowlist extension: whether a platform administrator granted it, its host patterns, and the deployment's egress policy mode.

```bash
everruns orgs egress-allowlist get [OPTIONS] --org <org>
```

| Flag | Description |
|---|---|
| `--org <ORG>` | Required. Organization public ID. |

Example:

```bash
# Check whether an organization may add hosts and which ones it added
everruns orgs egress-allowlist get --org org_01h9
```

## orgs egress-allowlist set

Replace the organization's outbound allowlist extension. Requires an organization admin and a grant from a platform administrator. Patterns must name public hosts: `example.com`, `*.example.com`, or an `https://example.com/path/` prefix.

```bash
everruns orgs egress-allowlist set [OPTIONS] --org <org> --patterns <patterns>
```

| Flag | Description |
|---|---|
| `--org <ORG>` | Required. Organization public ID. |
| `--patterns <PATTERNS>` | Required. The full list of host patterns; replaces the stored list. Comma-separated or a JSON array of strings. Repeatable. |

Example:

```bash
# Let agents reach your own API hosts under a curated egress policy
everruns orgs egress-allowlist set --org org_01h9 --patterns api.example.com --patterns '*.internal.example.com' --reason 'Agents call our ticketing API'
```

## orgs egress-allowlist grant set

Platform administrators only: allow or stop an organization extending the outbound allowlist (revoking keeps its patterns but stops enforcing them), and give or take back open egress (any public host the deny list allows, reads and writes). Omitted fields keep their value.

```bash
everruns orgs egress-allowlist grant set [OPTIONS] --org <org>
```

| Flag | Description |
|---|---|
| `--granted` | Allow or stop this organization extending the allowlist; omitted keeps it. |
| `--open` | Give or take back open egress; omitted keeps it. |
| `--org <ORG>` | Required. Organization public ID. |

Example:

```bash
# Allow an organization to extend the outbound allowlist (platform administrators)
everruns orgs egress-allowlist grant set --org org_01h9 --granted true --reason 'Approved in security review'

# Give a verified organization open egress (platform administrators)
everruns orgs egress-allowlist grant set --org org_01h9 --open true --reason 'Paid account, verified card'
```

## orgs provider-accounts list

List the organization's provider accounts (sandbox provider accounts such as Daytona or E2B, and web search accounts): id, name, provider, provider username, and timestamps. Metadata only, never credentials. Use an id as a Sandbox Template's connection or a web search account. Organization admins only.

```bash
everruns orgs provider-accounts list [OPTIONS] --org <org>
```

| Flag | Description |
|---|---|
| `--org <ORG>` | Required. Organization public ID. |

Example:

```bash
# Find the sandbox provider account to point a Sandbox Template at
everruns orgs provider-accounts list --org org_01h9
```
