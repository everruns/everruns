---
title: everruns observers
description: "Online scoring of live traces. CLI reference for everruns observers: usage, flags and examples."
sidebar:
  label: observers
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Online scoring of live traces.

| Command | What it does |
|---|---|
| [`observers backfill`](#observers-backfill) | Score past production traffic with an observer. |
| [`observers create`](#observers-create) | Create an observer (online scoring). |
| [`observers delete`](#observers-delete) | Archive an observer. |
| [`observers get`](#observers-get) | Get a single observer. |
| [`observers summary`](#observers-summary) | Live quality by agent, observer and day. |
| [`observers list`](#observers-list) | List observers. |
| [`observers update`](#observers-update) | Update an observer. |
| [`observers scores list`](#observers-scores-list) | List trace scores produced by an observer. |
| [`observers scores review`](#observers-scores-review) | Record whether an observer's score was right. |

## observers backfill

Score past production traffic with an observer.

```bash
everruns observers backfill [OPTIONS] --observer-id <observer_id>
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` | Most past events to read, newest first (1–5000). |
| `--observer-id <OBSERVER_ID>` | Required. Observer's prefixed public identifier. |
| `--sample` | Apply the observer's sampling rate. |
| `--since-hours <SINCE_HOURS>` | How far back to read, in hours (1–720). |

Example:

```bash
# Score the last day of traffic with a new observer
everruns observers backfill --observer-id observer_01h9 --since-hours 24 --reason 'Baseline the new rubric'
```

## observers create

Create an observer (online scoring).

```bash
everruns observers create [OPTIONS] --name <name> --scorers <scorers>
```

| Flag | Description |
|---|---|
| `--alert <ALERT>` |  |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--match <MATCH>` |  |
| `--name <NAME>` | Required. Human-readable name. |
| `--sampling-rate <SAMPLING_RATE>` | Fraction of matching turns to score (0.0–1.0). |
| `--scorers <SCORERS>` | Required. Scoring rules. |

Example:

```bash
# Score a sample of production turns automatically
everruns observers create --name helpfulness --sampling-rate 0.1 --scorers '[{"key":"mentions_docs","method":"rule","rule":{"type":"contains","text":"docs"}}]' --reason 'Track answer quality'
```

## observers delete

Archive an observer.

```bash
everruns observers delete [OPTIONS] --observer-id <observer_id>
```

| Flag | Description |
|---|---|
| `--observer-id <OBSERVER_ID>` | Required. Observer's prefixed public identifier. |

Example:

```bash
# Archive an observer you no longer want scoring turns
everruns observers delete --observer-id observer_01h9 --reason 'Replaced by a stricter observer'
```

## observers get

Get a single observer.

```bash
everruns observers get [OPTIONS] [OBSERVER_ID]
```

| Flag | Description |
|---|---|
| `--observer-id <OBSERVER_ID>` | Observer's prefixed public identifier. |

Example:

```bash
# Check an observer's scorers, match rules and sampling rate
everruns observers get observer_01h9
```

## observers summary

Live quality by agent, observer and day.

```bash
everruns observers summary [OPTIONS]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Only scores of this agent. |
| `--days <DAYS>` | Window in days (1–90). |
| `--observer-id <OBSERVER_ID>` | Only scores of this observer. |

Example:

```bash
# See how one agent's production answers scored this week
everruns observers summary --agent-id agent_01h9 --days 7
```

## observers list

List observers.

```bash
everruns observers list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived observers. |

Example:

```bash
# See which observers are scoring production sessions
everruns observers list
```

## observers update

Update an observer.

```bash
everruns observers update [OPTIONS] --observer-id <observer_id>
```

| Flag | Description |
|---|---|
| `--alert <ALERT>` |  |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--match <MATCH>` |  |
| `--name <NAME>` | Human-readable name. |
| `--observer-id <OBSERVER_ID>` | Required. Observer's prefixed public identifier. |
| `--sampling-rate <SAMPLING_RATE>` | Fraction of matching turns to score (0.0–1.0). |
| `--scorers <SCORERS>` |  |
| `--status <STATUS>` |  |

Example:

```bash
# Score fewer turns to cut judge cost
everruns observers update --observer-id observer_01h9 --sampling-rate 0.05 --reason 'Reduce scoring volume'
```

## observers scores list

List trace scores produced by an observer.

```bash
everruns observers scores list [OPTIONS] --observer-id <observer_id>
```

| Flag | Description |
|---|---|
| `--limit <LIMIT>` |  |
| `--observer-id <OBSERVER_ID>` | Required. Observer's prefixed public identifier. |
| `--offset <OFFSET>` |  |
| `--review-queue` | Only unreviewed completed scores, failures first (the review queue). |
| `--session-id <SESSION_ID>` | Session's prefixed public identifier. |

Example:

```bash
# Look at the scores an observer gave one session
everruns observers scores list --observer-id observer_01h9 --session-id session_01h9
```

## observers scores review

Record whether an observer's score was right.

```bash
everruns observers scores review [OPTIONS] --agrees [<agrees>] --observer-id <observer_id> --score-id <score_id>
```

| Flag | Description |
|---|---|
| `--agrees` | Required. True when the grader got it right. |
| `--note <NOTE>` | Optional note on why (at most 2000 characters). |
| `--observer-id <OBSERVER_ID>` | Required. Observer's prefixed public identifier. |
| `--score-id <SCORE_ID>` | Required. Score's prefixed public identifier. |

Example:

```bash
# Mark a failing score as a grader mistake
everruns observers scores review --observer-id observer_01h9 --score-id score_01h9 --agrees false --note 'The answer cited the refund policy' --reason 'Calibrate the grounded judge'
```
