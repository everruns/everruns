---
title: everruns evals
description: "Evaluation cases, runs, results and scores. CLI reference for everruns evals."
sidebar:
  label: evals
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Evaluation cases, runs, results and scores.

| Command | What it does |
|---|---|
| [`evals create`](#evals-create) | Create a new eval. |
| [`evals delete`](#evals-delete) | Delete an eval. |
| [`evals import-preflight`](#evals-import-preflight) | Report whether the caller can import eval results. |
| [`evals get`](#evals-get) | Get a single eval. |
| [`evals import`](#evals-import) | Import externally-executed eval results. |
| [`evals list`](#evals-list) | List evals. |
| [`evals update`](#evals-update) | Update an eval. |
| [`evals atif-import import`](#evals-atif-import-import) | Import ATIF trajectories as eval cases (upserted by name). |
| [`evals cases create`](#evals-cases-create) | Create an eval case. |
| [`evals cases delete`](#evals-cases-delete) | Delete an eval case. |
| [`evals cases get`](#evals-cases-get) | Get an eval case. |
| [`evals cases list`](#evals-cases-list) | List eval cases. |
| [`evals cases update`](#evals-cases-update) | Update an eval case. |
| [`evals runs cancel`](#evals-runs-cancel) | Cancel an eval run. |
| [`evals runs create`](#evals-runs-create) | Create an eval run. |
| [`evals runs get`](#evals-runs-get) | Get an eval run. |
| [`evals runs list`](#evals-runs-list) | List eval runs. |
| [`evals runs artifacts export`](#evals-runs-artifacts-export) | Export eval run artifacts as NDJSON. |
| [`evals runs dataset export`](#evals-runs-dataset-export) | Enqueue an async reward-labeled trajectory dataset export from a completed eval run. |
| [`evals runs dataset get`](#evals-runs-dataset-get) | Fetch an eval-run dataset export handle (status + NDJSON body). |
| [`evals runs results scores update`](#evals-runs-results-scores-update) | Update scores for one eval result. |
| [`evals runs scores bulk`](#evals-runs-scores-bulk) | Bulk update scores for all results in an eval run. |
| [`evals runs share create`](#evals-runs-share-create) | Mint a read-only share link for an eval run. |
| [`evals runs share get`](#evals-runs-share-get) | Whether an eval run has an active share link. |
| [`evals runs share revoke`](#evals-runs-share-revoke) | Revoke all share links for an eval run. |

## evals create

Create a new eval.

```bash
everruns evals create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--model-override <MODEL_OVERRIDE>` | Default model override applied to runs of this eval. |
| `--name <NAME>` | Required. Human-readable name. |
| `--tags <TAGS>` | Free-form tags attached to this resource. Repeatable. |
| `--target <TARGET>` |  |

Example:

```bash
# Start a regression suite aimed at one agent
everruns evals create --name support-regression --target '{"type":"session","agent_id":"agent_01h9"}' --reason 'Track support agent quality'
```

## evals delete

Delete an eval.

```bash
everruns evals delete [OPTIONS] --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |

Example:

```bash
# Remove an eval that is no longer tracked
everruns evals delete --eval-id eval_01h9 --reason 'Suite retired'
```

## evals import-preflight

Report whether the caller can import eval results.

```bash
everruns evals import-preflight [OPTIONS]
```

Example:

```bash
# Check whether this caller can import eval results
everruns evals import-preflight
```

## evals get

Get a single eval.

```bash
everruns evals get [OPTIONS] [EVAL_ID]
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Eval's prefixed public identifier. |

Example:

```bash
# Show an eval's target and settings
everruns evals get eval_01h9
```

## evals import

Import externally-executed eval results.

```bash
everruns evals import [OPTIONS] --evals <evals> --source <source>
```

| Flag | Description |
|---|---|
| `--evals <EVALS>` | Required. Evals and their case results in this run. |
| `--source <SOURCE>` | Required. Attribution for the external system that produced the run. |

Example:

```bash
# Record results from an eval harness that ran outside Everruns
everruns evals import --source '{"system":"mira","run_id":"run-42"}' --evals '[{"name":"support","cases":[{"name":"refund","target":{"provider":"openai","model":"gpt-5.1"},"status":"passed"}]}]' --reason 'Publish the nightly Mira run'
```

## evals list

List evals.

```bash
everruns evals list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived evals. |
| `--search <SEARCH>` | Case-insensitive name filter. |

Example:

```bash
# Find an eval by name when you do not know the id
everruns evals list --search support
```

## evals update

Update an eval.

```bash
everruns evals update [OPTIONS] --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--model-override <MODEL_OVERRIDE>` | Default model override applied to runs of this eval. |
| `--name <NAME>` | Human-readable name. |
| `--tags <TAGS>` | Free-form tags attached to this resource. Repeatable. |
| `--target <TARGET>` |  |

Example:

```bash
# Point an eval at a different model by default
everruns evals update --eval-id eval_01h9 --model-override gpt-5.1 --reason 'Move the suite to the new model'
```

## evals atif-import import

Import ATIF trajectories as eval cases (upserted by name).

```bash
everruns evals atif-import import [OPTIONS] --body <body> --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--body <BODY>` | Required. Raw ATIF payload (NDJSON or JSON). |
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |

Example:

```bash
# Turn recorded agent trajectories into eval cases
everruns evals atif-import import --eval-id eval_01h9 --body "$(cat trajectories.ndjson)" --reason 'Seed cases from production sessions'
```

## evals cases create

Create an eval case.

```bash
everruns evals cases create [OPTIONS] --conversation <conversation> --eval-id <eval_id> --name <name> --scorers <scorers>
```

| Flag | Description |
|---|---|
| `--artifacts <ARTIFACTS>` | Session files to capture after scoring completes. |
| `--conversation <CONVERSATION>` | Required. Input messages sent to the agent sequentially. |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--max-turns <MAX_TURNS>` | Maximum agent turns before the case stops. |
| `--name <NAME>` | Required. Human-readable name. |
| `--position <POSITION>` | Display order within the eval. |
| `--post <POST>` | Verification messages sent after conversation completes and session idles. |
| `--scorers <SCORERS>` | Required. Scoring rules applied to the case output. |
| `--tags <TAGS>` | Free-form tags attached to this resource. Repeatable. |
| `--target <TARGET>` |  |
| `--timeout-seconds <TIMEOUT_SECONDS>` | Per-case timeout in seconds. |

Example:

```bash
# Add a scripted conversation with a pass condition to an eval
everruns evals cases create --eval-id eval_01h9 --name fix-failing-test --conversation '[{"content":"Fix the failing test in src/lib.rs"}]' --scorers '[{"type":"contains","text":"tests pass"}]' --reason 'Cover the test-fix flow'
```

## evals cases delete

Delete an eval case.

```bash
everruns evals cases delete [OPTIONS] --case-id <case_id> --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--case-id <CASE_ID>` | Required. Eval case's prefixed public identifier. |
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |

Example:

```bash
# Drop a case that no longer reflects expected behavior
everruns evals cases delete --eval-id eval_01h9 --case-id evalcase_01h9 --reason 'Duplicate of fix-failing-test'
```

## evals cases get

Get an eval case.

```bash
everruns evals cases get [OPTIONS] --case-id <case_id> --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--case-id <CASE_ID>` | Required. Eval case's prefixed public identifier. |
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |

Example:

```bash
# Read one case's conversation and scorers
everruns evals cases get --eval-id eval_01h9 --case-id evalcase_01h9
```

## evals cases list

List eval cases.

```bash
everruns evals cases list [OPTIONS] --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |

Example:

```bash
# See which cases an eval contains
everruns evals cases list --eval-id eval_01h9
```

## evals cases update

Update an eval case.

```bash
everruns evals cases update [OPTIONS] --case-id <case_id> --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--artifacts <ARTIFACTS>` | Session files to capture after scoring completes. |
| `--case-id <CASE_ID>` | Required. Eval case's prefixed public identifier. |
| `--conversation <CONVERSATION>` | Input messages sent to the agent sequentially. |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--max-turns <MAX_TURNS>` | Maximum agent turns before the case stops. |
| `--name <NAME>` | Human-readable name. |
| `--position <POSITION>` | Display order within the eval. |
| `--post <POST>` | Verification messages sent after conversation completes. |
| `--scorers <SCORERS>` | Scoring rules applied to the case output. |
| `--tags <TAGS>` | Free-form tags attached to this resource. Repeatable. |
| `--target <TARGET>` |  |
| `--timeout-seconds <TIMEOUT_SECONDS>` | Per-case timeout in seconds. |

Example:

```bash
# Give a slow case more turns
everruns evals cases update --eval-id eval_01h9 --case-id evalcase_01h9 --max-turns 20 --reason 'Case timed out at the old limit'
```

## evals runs cancel

Cancel an eval run.

```bash
everruns evals runs cancel [OPTIONS] --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Stop a run that was started with the wrong settings
everruns evals runs cancel --eval-id eval_01h9 --run-id evalrun_01h9 --reason 'Wrong model override'
```

## evals runs create

Create an eval run.

```bash
everruns evals runs create [OPTIONS] --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--model-override <MODEL_OVERRIDE>` | Model override for this run. |
| `--target <TARGET>` |  |

Example:

```bash
# Run every case in an eval, optionally on a different model
everruns evals runs create --eval-id eval_01h9 --model-override gpt-5.1 --reason 'Check the suite on the new model'
```

## evals runs get

Get an eval run.

```bash
everruns evals runs get [OPTIONS] --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Check a run's status and per-case results
everruns evals runs get --eval-id eval_01h9 --run-id evalrun_01h9
```

## evals runs list

List eval runs.

```bash
everruns evals runs list [OPTIONS] --eval-id <eval_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |

Example:

```bash
# Find a past run to compare against
everruns evals runs list --eval-id eval_01h9
```

## evals runs artifacts export

Export eval run artifacts as NDJSON.

```bash
everruns evals runs artifacts export [OPTIONS] --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Download the files a run captured, one NDJSON line per result
everruns evals runs artifacts export --eval-id eval_01h9 --run-id evalrun_01h9
```

## evals runs dataset export

Enqueue an async reward-labeled trajectory dataset export from a completed eval run.

```bash
everruns evals runs dataset export [OPTIONS] --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--filters <FILTERS>` | Selection filters applied per case. |
| `--format <FORMAT>` | Output schema for the exported dataset. One of `trajectory`, `sft`, `atif`. |
| `--redaction <REDACTION>` | Redaction controls. |
| `--run-id <RUN_ID>` | Required. Completed eval run's prefixed public identifier. |

Example:

```bash
# Build a training dataset from a finished run
everruns evals runs dataset export --eval-id eval_01h9 --run-id evalrun_01h9 --format sft --reason 'Fine-tune on the passing cases'
```

## evals runs dataset get

Fetch an eval-run dataset export handle (status + NDJSON body).

```bash
everruns evals runs dataset get [OPTIONS] --dataset-id <dataset_id> --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--dataset-id <DATASET_ID>` | Required. Dataset export's prefixed public identifier. |
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Poll a dataset export until it completes, then read its NDJSON
everruns evals runs dataset get --eval-id eval_01h9 --run-id evalrun_01h9 --dataset-id evaldataset_01h9
```

## evals runs results scores update

Update scores for one eval result.

```bash
everruns evals runs results scores update [OPTIONS] --eval-id <eval_id> --result-id <result_id> --run-id <run_id> --scores <scores>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--metadata <METADATA>` | Free-form metadata attached to this resource. |
| `--result-id <RESULT_ID>` | Required. Eval case result's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |
| `--scores <SCORES>` | Required. Externally computed scores to store on the result. |
| `--status <STATUS>` |  |

Example:

```bash
# Override one result's scores after a manual review
everruns evals runs results scores update --eval-id eval_01h9 --run-id evalrun_01h9 --result-id evalresult_01h9 --scores '[{"pass":false,"value":0.0,"reason":"Wrong refund amount"}]' --status failed --reason 'Manual review'
```

## evals runs scores bulk

Bulk update scores for all results in an eval run.

```bash
everruns evals runs scores bulk [OPTIONS] --eval-id <eval_id> --results <results> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--metadata <METADATA>` | Free-form metadata attached to this resource. |
| `--results <RESULTS>` | Required. Per-result score updates applied together. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Attach scores from an external grader to every result in a run
everruns evals runs scores bulk --eval-id eval_01h9 --run-id evalrun_01h9 --results '[{"result_id":"evalresult_01h9","scores":[{"pass":true,"value":1.0,"reason":"Matches the reference"}],"status":"passed"}]' --reason 'Apply grader output'
```

## evals runs share create

Mint a read-only share link for an eval run.

```bash
everruns evals runs share create [OPTIONS] --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Mint a read-only link to show a run's results to someone outside the org
everruns evals runs share create --eval-id eval_01h9 --run-id evalrun_01h9 --reason 'Share results with the vendor'
```

## evals runs share get

Whether an eval run has an active share link.

```bash
everruns evals runs share get [OPTIONS] --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Check whether a run is currently shared
everruns evals runs share get --eval-id eval_01h9 --run-id evalrun_01h9
```

## evals runs share revoke

Revoke all share links for an eval run.

```bash
everruns evals runs share revoke [OPTIONS] --eval-id <eval_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--eval-id <EVAL_ID>` | Required. Eval's prefixed public identifier. |
| `--run-id <RUN_ID>` | Required. Eval run's prefixed public identifier. |

Example:

```bash
# Turn off every share link for a run
everruns evals runs share revoke --eval-id eval_01h9 --run-id evalrun_01h9 --reason 'Vendor review finished'
```
