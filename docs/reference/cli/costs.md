---
title: everruns costs
description: "What usage cost the organization, its price list and cost settings. CLI reference for everruns costs: usage, flags and examples."
sidebar:
  label: costs
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

What usage cost the organization, its price list and cost settings.

| Command | What it does |
|---|---|
| [`costs rates create`](#costs-rates-create) | Add a rate to the organization's price list. |
| [`costs rates list`](#costs-rates-list) | List the organization's price list, history included. |
| [`costs rates retire`](#costs-rates-retire) | End a rate from a date. |
| [`costs settings get`](#costs-settings-get) | Read the organization's cost settings. |
| [`costs settings update`](#costs-settings-update) | Set the timezone usage policy windows reset in. |
| [`costs summary get`](#costs-summary-get) | Summarize what usage cost the organization. |

## costs rates create

Add a rate to the organization's price list. A rate is a fixed price per unit or a multiple of the list price; rates are never edited, a change is a new rate from a later date.

```bash
everruns costs rates create [OPTIONS] --meter <meter>
```

| Flag | Description |
|---|---|
| `--effective-from <EFFECTIVE_FROM>` | When it starts; now when absent. |
| `--list-price-multiplier <LIST_PRICE_MULTIPLIER>` | Multiple of the provider's or catalog price. |
| `--match <MATCH>` | Which usage it applies to (see `CostRate.match`). |
| `--meter <METER>` | Required. `model.tokens` or `sandbox.time`. |
| `--unit-price-usd <UNIT_PRICE_USD>` | Fixed USD per million tokens (model tokens) or per hour (sandboxes). |

Example:

```bash
# Record a negotiated 20% discount on OpenAI
everruns costs rates create --meter model.tokens --match '{"provider":"openai"}' --list-price-multiplier 0.8 --reason 'Enterprise discount'
```

## costs rates list

List the organization's price list, history included.

```bash
everruns costs rates list [OPTIONS]
```

Example:

```bash
# See which rates price your agents' usage
everruns costs rates list
```

## costs rates retire

End a rate from a date. Past costs keep the rate they used; usage after the date falls back to the provider's or catalog price.

```bash
everruns costs rates retire [OPTIONS] --rate-id <rate_id>
```

| Flag | Description |
|---|---|
| `--effective-from <EFFECTIVE_FROM>` | When it ends; now when absent. |
| `--rate-id <RATE_ID>` | Required. Rate id (`rate_...`) whose match to retire. |

Example:

```bash
# Stop applying a discount that expired
everruns costs rates retire --rate-id rate_01h9 --reason 'Contract ended'
```

## costs settings get

Read the organization's cost settings.

```bash
everruns costs settings get [OPTIONS]
```

Example:

```bash
# See which timezone daily and monthly limits reset in
everruns costs settings get
```

## costs settings update

Set the timezone usage policy windows reset in. Windows already running keep their start; the next boundary uses the new timezone.

```bash
everruns costs settings update [OPTIONS]
```

| Flag | Description |
|---|---|
| `--timezone <TIMEZONE>` | IANA timezone, or null for UTC. |

Example:

```bash
# Reset monthly limits at midnight in Austin
everruns costs settings update --timezone America/Chicago --reason 'Team is in Austin'
```

## costs summary get

Summarize what usage cost the organization.

```bash
everruns costs summary get [OPTIONS]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Only this agent (`agent_...`). |
| `--from <FROM>` | Window start (inclusive). |
| `--to <TO>` | Window end (exclusive). |

Example:

```bash
# See this month's cost by agent and source
everruns costs summary get
```
