---
name: tune-cli-help
description: Tune `everruns` command help (descriptions, examples, group summaries) against the platform-capability eval's friction numbers, keeping only edits that cut tool calls or rejected guesses without regressing a case. Use when asked to improve CLI help, lower tool calls to finish, act on a friction report, or run the help-tuning loop.
metadata:
  internal: true
user-invocable: true
---

# Tune CLI help

Models do not know the `everruns` command tree from training. Every `--help` read and every
rejected guess is a tool call the help text could have saved. The goal is fewer calls to finish
each `cli-` case, measured, across more than one model family.

Read [`evals/platform-capability/README.md`](../../../evals/platform-capability/README.md)
("Friction" and "Running it without a server") first. It owns what the numbers mean. This skill
owns the loop.

## Inputs

- A model key (`OPENROUTER_API_KEY`, through Doppler). No server, database or stack: the cases run
  against the offline control plane, which uses the shipped prompt, help and parser.
- At least two model families as targets, for example one OpenAI and one non-OpenAI model on
  OpenRouter. Help tuned to one model's habits is not better help.

## Loop

1. **Baseline.** From `evals/platform-capability`, build once (`cargo build --locked`), then run
   the command-line cases offline with a few trials per target and keep the report:

   ```bash
   export EVERRUNS_EVAL_MODE=offline EVERRUNS_EVAL_TRIALS=3
   export EVERRUNS_EVAL_TARGETS=openai/gpt-5.5,meta/muse-spark-1.3-contributor
   doppler run --command './target/debug/platform_capability --run --filter cli- \
     --friction-report /tmp/friction-before.jsonl' | tee /tmp/before.txt
   ```

   Record per case and overall: mean calls, help reads, rejected, passes.
2. **Read the friction report.** One JSON line per run. Drop lines with a non-null `error` (the
   run never reached a model). Look for, in order of cost:
   - the same `rejected` command across runs and models: the error text it got and what the model
     typed next show which flag, verb or noun it expected;
   - help pages read before the first real command: a case that needs three pages to find one
     command has a group summary or a description that does not say what the command is for;
   - `kind: "unknown"` commands: flat wire names or invented commands, a sign the prompt's map or
     a node summary does not lead to the tree spelling.
3. **Change the help where they stumbled**, smallest edit first:
   - a group's one-line summary: `NODE_ABOUT` in `crates/cli-contract/src/mapper.rs`. Placeholder
     summaries (`<noun> commands`) are the usual cause of extra help reads;
   - a command's description, or its intent-plus-command example:
     `cli = CliRoute::new(..).with_examples(&[CliExample::new(intent, command)])` on the command in
     `crates/server/src/domains/**/commands.rs`. An example must run as written: the eval's
     contract test and `everruns-cli` both check examples against the real flags.
   Do not add flags, aliases or prompt text to fit a model's guess. The tree is the contract; help
   is how it is found.
4. **Regenerate the artifacts** the eval and the CLI mount, then confirm the guards pass:

   ```bash
   UPDATE_CLI_CONTRACT_COMMANDS=1 cargo test -p everruns-server --lib the_checked_in_contract_matches_inventory
   UPDATE_EVAL_CATALOG=1 cargo test -p everruns-server --lib the_eval_
   UPDATE_CLI_CONTRACT=1 cargo test -p everruns-cli
   (cd evals/platform-capability && cargo test --locked)
   ```

5. **Rerun** step 1 with the same targets and trials into `friction-after.jsonl`.
6. **Keep or revert.** Keep a change only when, on every target family, mean calls or mean
   rejections drop and no case's pass count drops. Run-to-run spread is about one trial in three
   per case (see the README), so a move inside that on one target is noise: rerun with more trials
   or revert. Revert anything that helps one family and hurts another.
7. **Repeat** from step 2 on what is left, one theme per iteration so a regression has one cause.

## Constraints

- Help is for people too. Keep every description one readable sentence and every example
  something a person would type; no keyword stuffing, no model-specific phrasing, no long lists.
  Bounded help is a design property of the tree ([`knowledge/execution/command-tree.md`](../../../knowledge/execution/command-tree.md)).
- Do not change the dataset, budgets or scorers in the same change as the help: the numbers must
  compare like for like.
- Only the offline cases run here. Paid runs are the person's call; say what a run will cost
  (cases x trials x targets) before starting one.

## Done when

The PR carries the before and after table (per case and overall, per target) from `--run`, names
the help edits that moved which case, lists edits tried and reverted, and the regenerated
artifacts are committed with the guards green.
