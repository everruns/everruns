---
title: Evals
description: Define, run, and track behavioral tests for Agents. Each case runs a real session and is scored.
sidebar:
  label: Evals
appliesTo: [platform, cloud]
---

Evals let you define, run, and track **behavioral tests** for your Agents. An Eval is a named collection of cases; each case sends messages to a fresh session and scores the result. Use them to compare runs across models and catch regressions after a prompt change.

Because every case runs a **real session** against the target agent and harness, failures are fully debuggable, click into the session to see the conversation, tool calls, and events exactly as they happened.

## Concepts

| Entity | What it is |
|---|---|
| **Eval** | Top-level, org-scoped collection of cases. Follows the standard `active → archived → deleted` lifecycle. |
| **EvalCase** | A single test: input messages or a simulated user, tool fixtures, scoring rules, execution bounds, and optional artifact collection. |
| **EvalRun** | One execution of an eval's cases against a target, producing per-case results. |
| **EvalTarget** | How a session is instantiated from a Harness, Agent, model, and system prompt. |

Targets resolve in order `EvalRun → EvalCase → Eval → org default harness`, so you can run the same cases against different models per run or override per case.

## How a case runs

1. A fresh session is created from the resolved target.
2. The case's `conversation` messages are delivered sequentially (multi-turn supported).
3. Optional `post` messages run after the session idles, for example, executing a test script.
4. Optional `artifacts` capture named session files.
5. `scorers` grade the result, returning `0.0`–`1.0` for nuanced pass/fail.

Eval runs are durable workflows: they reuse the same execution engine as production sessions, so a worker restart mid-run does not lose progress.

## Multistep cases

A case can test a whole conversation instead of one exchange.

- **Simulated user**: give the case a persona, a goal and, optionally, when the user is done. After the scripted messages (there may be none), a model writes each next user message from the conversation so far, until it decides the goal is met or reaches the turn limit (default 8, at most 30). It runs on your organization's models, the default or one you pick, and its spend counts toward the run's cap. It can play the user as a virtual user, so the agent sees that end user's identity and connections. Each result records how the conversation ended and which model played the user.
- **Tool fixtures**: tools the agent gets in this case with a canned response. Every call is answered with the response and kept in the trace, so the case never reaches a real system. The `tool_called` grader's `arguments_contain` checks what the agent sent, for example the right order number.

Judge and decision graders of a multistep case see the whole conversation. `post` messages do not combine with a simulated user.

```bash
everruns evals cases create --eval-id eval_01h9 --name "Refund, angry customer" \
  --simulated-user '{"persona":"Shop owner charged twice for the Pro plan, writes short messages","goal":"Get the duplicate charge refunded","done_when":"A refund ticket exists with order ORD-48213"}' \
  --tool-fixtures '[{"name":"create_ticket","description":"Open a helpdesk ticket","response":{"ticket":"T-1042"}}]' \
  --scorers '[{"type":"tool_called","tool":"create_ticket","arguments_contain":"ORD-48213"}]' \
  --reason 'Cover the refund conversation'
```

On a case's page, **Try once** runs only that case and opens the run.

## Experiments

An experiment answers "is this change better?". It runs one dataset (an eval's cases, at the version it has now) against two to four variants, each a model or target, with several trials per case. The first variant is the baseline. Each variant is an ordinary run, so trials, spending caps and durable execution work the same way; the experiment's cap is split evenly between its variants.

The experiment page compares variants case by case. It names the cases a variant fixed and the cases it broke, and gives a verdict: a likely improvement or regression when the pass-rate change is outside run-to-run noise (a 95% interval from a paired bootstrap over cases), otherwise no clear difference. Fewer than three cases give no verdict.

For CI, the gate fails when a variant misses a bar:

```bash
everruns evals experiments create --eval-id eval_01h9 \
  --variants '[{"label":"luna","model_override":"gpt-6-luna"},{"label":"sol","model_override":"gpt-6.1-sol"}]' \
  --max-cost-usd 10
everruns evals experiments gate --eval-id eval_01h9 --experiment-id evalexp_01h9 \
  --min-pass-rate 0.9 --no-regression true
```

The gate answers 409 while runs are going and 422 when the variant misses the bar, so the command exits non-zero.

## In the UI

**Quality → Evals** opens on an overview of every agent with a dataset: its pass rate, the trend over recent runs, open findings and the last activity. Tabs hold datasets, experiments, observers, findings from session detectors and classifiers, and the graders a case can use. **Run experiment** starts one from any dataset. Each agent's page has an **Evals** tab with the same view for that agent.

## SWE-bench Lite

Beyond user-facing behavioral evals, Everruns ships a **SWE-bench Lite** harness for measuring coding-agent performance against the standard benchmark, using the same eval machinery (`post` verification messages run the test scripts that determine pass/fail).

## Related

- [Harnesses](/features/harnesses/), what an eval target runs
- [Change History](/features/change-history/), every change to an agent and how to restore an earlier configuration
