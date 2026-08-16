---
title: Evals
description: Define, run, and track behavioral tests for your agents. Each eval case runs a real session and is scored, so you can compare models, catch regressions, and gate App publishes on pass rates.
sidebar:
  label: Evals
---

Evals let you define, run, and track **behavioral tests** for your agents. An Eval is a named collection of cases; each case sends messages to a fresh session and scores the result. Use them to compare runs across models, catch regressions after a prompt change, and gate App publishes on pass rates.

Because every case runs a **real session** against the target agent and harness, failures are fully debuggable, click into the session to see the conversation, tool calls, and events exactly as they happened.

## Concepts

| Entity | What it is |
|---|---|
| **Eval** | Top-level, org-scoped collection of cases. Follows the standard `active → archived → deleted` lifecycle. |
| **EvalCase** | A single test: input messages, scoring rules, execution bounds, and optional artifact collection. |
| **EvalRun** | One execution of an eval's cases against a target, producing per-case results. |
| **EvalTarget** | How a session is instantiated, either a `session` setup (harness, agent, model, system prompt) or a deployed `app`. |

Targets resolve in order `EvalRun → EvalCase → Eval → org default harness`, so you can run the same cases against different models per run or override per case.

## How a case runs

1. A fresh session is created from the resolved target.
2. The case's `conversation` messages are delivered sequentially (multi-turn supported).
3. Optional `post` messages run after the session idles, for example, executing a test script.
4. Optional `artifacts` capture named session files.
5. `scorers` grade the result, returning `0.0`–`1.0` for nuanced pass/fail.

Eval runs are durable workflows: they reuse the same execution engine as production sessions, so a worker restart mid-run does not lose progress.

## SWE-bench Lite

Beyond user-facing behavioral evals, Everruns ships a **SWE-bench Lite** harness for measuring coding-agent performance against the standard benchmark, using the same eval machinery (`post` verification messages run the test scripts that determine pass/fail).

## Related

- [Harnesses](/features/harnesses/), what an eval target runs
- [Apps](/features/apps/), gating a publish on eval results
