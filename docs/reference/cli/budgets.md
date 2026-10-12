---
title: everruns budgets
description: "Spending limits for sessions, agents, users and organizations. CLI reference for everruns budgets."
sidebar:
  label: budgets
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Spending limits for sessions, agents, users and organizations.

| Command | What it does |
|---|---|
| [`budgets check`](#budgets-check) | Check budget status for a session-scoped budget. |
| [`budgets create`](#budgets-create) | Create a budget for a subject (session, agent, user, org). |
| [`budgets delete`](#budgets-delete) | Delete a budget. |
| [`budgets get`](#budgets-get) | Get a single budget by ID. |
| [`budgets list`](#budgets-list) | List budgets. |
| [`budgets update`](#budgets-update) | Update a budget limit, status, or metadata. |
| [`budgets ledger list`](#budgets-ledger-list) | List ledger entries for a budget. |
| [`budgets top-up top`](#budgets-top-up-top) | Add credits to a budget. |

## budgets check

Check budget status for a session-scoped budget.

```bash
everruns budgets check [OPTIONS] --budget-id <budget_id>
```

| Flag | Description |
|---|---|
| `--budget-id <BUDGET_ID>` | Required. Budget's prefixed public identifier. |

Example:

```bash
# Check whether a budget still has room before starting expensive work
everruns budgets check --budget-id bdgt_01h9
```

## budgets create

Create a budget for a subject (session, agent, user, org). Sets a spending cap in the given currency.

```bash
everruns budgets create [OPTIONS] --currency <currency> --limit <limit> --subject-id <subject_id> --subject-type <subject_type>
```

| Flag | Description |
|---|---|
| `--currency <CURRENCY>` | Required. Unit in which usage and the limit are measured. |
| `--limit <LIMIT>` | Required. Hard spending ceiling for the budget. |
| `--metadata <METADATA>` | Free-form metadata attached to this resource. |
| `--period <PERIOD>` |  |
| `--soft-limit <SOFT_LIMIT>` | Optional threshold that triggers a warning or pause before exhaustion. |
| `--subject-id <SUBJECT_ID>` | Required. Public identifier of the constrained resource. |
| `--subject-type <SUBJECT_TYPE>` | Required. Kind of resource constrained by the budget. |

Example:

```bash
# Cap an agent's spend with an early warning before the hard limit
everruns budgets create --subject-type agent --subject-id agent_01h9 --currency usd --limit 100 --soft-limit 80 --reason 'Monthly spend cap for the triage agent'
```

## budgets delete

Delete a budget.

```bash
everruns budgets delete [OPTIONS] [BUDGET_ID]
```

| Flag | Description |
|---|---|
| `--budget-id <BUDGET_ID>` | Budget's prefixed public identifier. |

Example:

```bash
# Remove a cap that no longer applies
everruns budgets delete bdgt_01h9 --reason 'Agent retired'
```

## budgets get

Get a single budget by ID.

```bash
everruns budgets get [OPTIONS] [BUDGET_ID]
```

| Flag | Description |
|---|---|
| `--budget-id <BUDGET_ID>` | Budget's prefixed public identifier. |

Example:

```bash
# Inspect a budget's limit, balance and status
everruns budgets get bdgt_01h9
```

## budgets list

List budgets. Filter by subject_type and subject_id.

```bash
everruns budgets list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--subject-id <SUBJECT_ID>` | Only budgets on this subject's prefixed public identifier. |
| `--subject-type <SUBJECT_TYPE>` | Only budgets on this kind of subject (session, agent, user, org, ...). |

Example:

```bash
# Find the budgets that apply to one agent
everruns budgets list --subject-type agent --subject-id agent_01h9
```

## budgets update

Update a budget limit, status, or metadata.

```bash
everruns budgets update [OPTIONS] --budget-id <budget_id>
```

| Flag | Description |
|---|---|
| `--budget-id <BUDGET_ID>` | Required. Budget's prefixed public identifier. |
| `--limit <LIMIT>` | Maximum number of items returned in this page. |
| `--metadata <METADATA>` | Free-form metadata attached to this resource. |
| `--soft-limit <SOFT_LIMIT>` | Replacement soft threshold, or null to remove it. |
| `--status <STATUS>` | Current lifecycle status. |

Example:

```bash
# Raise a budget's hard limit
everruns budgets update --budget-id bdgt_01h9 --limit 150 --reason 'Higher volume expected'
```

## budgets ledger list

List ledger entries for a budget.

```bash
everruns budgets ledger list [OPTIONS] --budget-id <budget_id>
```

| Flag | Description |
|---|---|
| `--budget-id <BUDGET_ID>` | Required. Budget's prefixed public identifier (a path parameter). |
| `--limit <LIMIT>` | Maximum number of items returned in this page. |
| `--offset <OFFSET>` | Zero-based offset into the result set. |

Example:

```bash
# Audit what drew down a budget, newest entries first
everruns budgets ledger list --budget-id bdgt_01h9 --limit 20
```

## budgets top-up top

Add credits to a budget. Reactivates exhausted or paused budgets if balance becomes positive.

```bash
everruns budgets top-up top [OPTIONS] --amount <amount> --budget-id <budget_id>
```

| Flag | Description |
|---|---|
| `--amount <AMOUNT>` | Required. Credits to add, in the budget's currency. |
| `--budget-id <BUDGET_ID>` | Required. Budget's prefixed public identifier. |
| `--description <DESCRIPTION>` | Human-readable description. |

Example:

```bash
# Add credits to an exhausted budget so work can continue
everruns budgets top-up top --budget-id bdgt_01h9 --amount 25 --description 'Extra allowance for the launch week' --reason 'Launch week'
```
