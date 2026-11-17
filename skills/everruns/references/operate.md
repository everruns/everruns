# Operate an Everruns organization

Everything here is read-only unless the user asks for a change, so run it with
MCP `query` or the CLI. Commands are the `everruns` command language (see the
[cli.md](cli.md)); check flags with `--help`.

Start broad with a report, then drill into the agents and sessions it points at.

## Usage and spend: reports

```bash
everruns reports catalog get          # datasets, dimensions, measures, filters
everruns reports query run --help     # one query: dataset, measures, dimensions, filters, time range
```

The catalog is the source of truth; read it before writing a query. The
datasets today:

| Dataset | Answers |
|---|---|
| `llm_generations` | Tokens, latency, time to first token, errors by model, provider, agent or day |
| `tool_calls` | Which tools run, how often, and how often they fail |
| `turns`, `sessions` | Volume and outcomes of agent work |
| `budget_postings` | Money: debits and credits against budgets |
| `capability_usage` | Which capabilities are actually used |

Group by `agent_name` or `model` and by `day` for trends. Saved reports
(`everruns reports saved list`) are ones the organization already relies on:
check them before building a new query.

Report numbers with their time range and units, and say which dataset they
came from.

## Budgets

```bash
everruns budgets list --help          # limits per session, agent, user or organization
everruns budgets ledger list --help   # what was charged and when
```

Creating a budget, changing a limit or topping up spends or restricts money:
confirm the subject, amount and currency with the user first, then use
`execute` with `--reason`.

## What is broken

1. `everruns health-issues list`: problems Everruns already detected (provider
   credentials, failing integrations). `everruns health-issues get <id>` explains one.
2. Failing work: query `turns` or `llm_generations` grouped by `agent_name` for
   errors over the last day, then open a failing session
   ([debug-session.md](debug-session.md)).
3. Per-agent checks: `everruns agents health-checks list --help`.

## Who changed what

- `everruns history org --help`: every recorded change across the
  organization, newest first, with the reason given.
- `everruns history list <id>` and `everruns history diff --help` for one entity.
- `everruns orgs audit-logs list --help`: sign-ins, membership and other
  security events.

When a behavior change lines up with a recorded change, say so with both
timestamps. Rolling back (`history restore`) is a change: ask first.

## Models and providers

- `everruns models list`, and the organization default model.
- `everruns providers list --help`: which model providers are configured.
  Provider keys are entered by the user on the **Providers** page, never through you.

## Reporting back

Lead with the answer (a number, a list of broken things, the change that
explains it), then the evidence: the query or command, the time range, and
links to the agents and sessions involved.
