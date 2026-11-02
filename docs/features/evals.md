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

## Observers: scoring live traffic

Datasets test an agent before it ships. An **observer** scores a sample of real sessions after it ships. It matches sessions by agent, harness or tag, samples a share of them, and grades each sampled slice with the same graders a case uses, including LLM judges and decision models. Scoring runs in the background and never slows a turn.

Each grader on an observer picks what it grades:

| Scope | Grades | When |
|-------|--------|------|
| Answers (`turn`) | One answer: the user's message, the reply, the tool calls of that turn | When the turn completes |
| Conversations (`session`) | The whole conversation as a transcript | 10 minutes after the session goes idle; a new message re-arms it |
| Tools (`tool`) | One tool call: its arguments and its result or error, optionally only for the tools you list | When the call completes |

Judge spend is charged to the org and the agent, never to the person, channel, schedule or session it watched, so monitoring cannot use up an end user's limits.

**Alerts.** An observer with an `alert` opens a health issue, and notifies the authors of the agents it watches, when its pass rate over the last `window_hours` falls under `min_pass_rate`. The window needs `min_scored` scores first, so a handful of samples cannot raise it. The issue resolves itself when the rate recovers.

```bash
everruns observers update --observer-id observer_01h9 \
  --alert '{"min_pass_rate": 0.8, "window_hours": 24, "min_scored": 20}' \
  --reason 'Page me when support answers slip'
```

**Reviewing scores.** A grader can be wrong. The review queue lists completed scores nobody has checked yet, failures first, then a random sample of passes. Record whether the grader got it right:

```bash
everruns observers scores list --observer-id observer_01h9 --review-queue true
everruns observers scores review --observer-id observer_01h9 --score-id score_01h9 \
  --agrees false --note 'The answer did cite the refund policy' --reason 'Calibrate the judge'
```

**Backfill.** A new observer only sees traffic from now on. Backfill scores the last hours of past sessions the same way, sampling included unless you pass `--sample false`:

```bash
everruns observers backfill --observer-id observer_01h9 --since-hours 24 \
  --reason 'Baseline the new rubric'
```

**Live quality.** `GET /v1/observers/summary` (`everruns observers summary`) returns the pass rate of completed scores over the last `days`, in total and by agent, observer and day.

## Findings

Findings are health issues that detectors and classifiers raise about sessions and schedules: a tool failing in a loop, a spend spike, an agent that looks stuck, instructions followed from a tool result, suspected abuse, a runaway schedule. They notify the agent's authors. Hard limits stop the work (a repeating schedule at 48 runs, a budget at zero); a finding itself only stops something when your self-healing policy says so.

The **Inspector** writes up the serious ones (classifier findings and runaway schedules) within about 15 minutes, using your organization's default model: what happened in a few plain sentences, how serious it is, and one next step. It files each finding under a group of similar ones, so ten sessions with the same problem show as one row with a count. It keeps its write-up, not the session's text, and its cost counts against the org and agent budgets.

For an agent stuck in a loop, one that followed an injected instruction, or a runaway schedule, the Inspector may also suggest a **fix**: a short addition to the agent's instructions that would have prevented it. Nothing changes until a person acts. **Check against evals** runs the agent's eval with and without the addition as an experiment; once the check shows no case broke, **Approve** adds the text to the agent's instructions as a new revision you can restore from its history. A fix checked against instructions that have changed since needs another check. Suspected abuse never gets a fix.

```bash
everruns evals fixes list --agent-id agent_01933b5a000070008000000000000001 --open
everruns evals fixes check --fix-id fix_0193... --max-cost-usd 2 --reason 'Before applying the stop rule'
everruns evals fixes apply --fix-id fix_0193... --reason 'Stop rule checked on the claims dataset'
```

**Self-healing.** Settings → Limits, "When a finding opens", sets what Everruns does on its own. By default a schedule whose runs ended the same way 24 times in a row is paused at once, instead of at the 48-run stop: the finding says "Done automatically", the schedule's owner and the agent's authors are told, and **Resume schedule** undoes it. Choose **Offer to pause it** to get a **Pause schedule** button on the finding instead, or **Do nothing**. Changes to an agent are never applied automatically; you can only turn the suggestions off.

```bash
everruns health-issues actions list --issue-id 550e8400-e29b-41d4-a716-446655440000
everruns health-issues actions undo --issue-id 550e8400-e29b-41d4-a716-446655440000 --action-id fact_0193... --reason 'Still waiting on the claim'
```

**Dismiss** closes a finding that needs nothing and keeps your reason. A dismissed session finding stays closed even if a detector flags the same session again:

```bash
everruns health-issues dismiss --issue-id 550e8400-e29b-41d4-a716-446655440000 \
  --note 'Expected: nightly load test'
```

Operators of a deployment can list open suspected-abuse findings in every organization with `everruns system detections`. They are leads for a person to confirm; nothing is suspended automatically.

## Eval Builder

The **Eval Builder** proposes cases for an agent's datasets. Ask it with **Propose cases** on the agent's **Evals** tab, or from the CLI:

```bash
everruns evals proposals create --agent-id agent_01933b5a000070008000000000000001
everruns evals proposals list --agent-id agent_01933b5a000070008000000000000001
```

It reads the agent's instructions and tools, a sample of its sessions from the last two weeks (sessions with findings or failing observer scores first), and the cases its datasets already have. It then suggests up to six cases covering what the agent is for, edge cases from its instructions, failures seen in real sessions, and adversarial attempts such as instructions injected into a tool result. Each proposal says why it matters, links the sessions behind it, and comes with a check: a decision question, a judge rubric, expected text or a tool call. Conversations that need back and forth come as simulated cases.

Nothing is added until you accept a proposal into a dataset for that agent, and the new case is recorded in change history like any other. Dismissed proposals, and cases the datasets already have, are not offered again:

```bash
everruns evals proposals accept --proposal-id evalprop_01933b5a000070008000000000000001 \
  --eval-id eval_01933b5a000070008000000000000001 --reason 'Cover refunds over the limit'
everruns evals proposals dismiss --proposal-id evalprop_01933b5a000070008000000000000002 \
  --note 'Out of scope for this agent'
```

It runs on your organization's default model and reads only that agent's sessions in your organization. Its cost counts against the org and agent budgets.

## Mira

[Mira](https://github.com/everruns/mira) is the code-first eval runner: studies in code, model matrices, trials, and any CLI or SDK as the subject. It shares its formats with Everruns, so one dataset runs in either.

A dataset exports as Mira samples and imports from them. Each case becomes one sample named after the case; settings Mira has no field for (scorers, post messages, simulated user, tool fixtures, limits) ride under `metadata.everruns`, so an export imports back unchanged.

```bash
everruns evals samples export --eval-id eval_01h9 | jq -c '.samples[]' > cases.jsonl
everruns evals samples import --eval-id eval_01h9 --samples "$(jq -s . cases.jsonl)"
```

Cases are matched by name, so importing again updates them instead of adding copies. A sample written for Mira has no `metadata.everruns`: its `expected` text becomes a `contains` scorer. Attachments and seeded files have no case field, and the import report lists the samples that had them.

A Mira run published to Everruns can join an experiment. Publish with an experiment name and a variant label, and each eval's run becomes that variant of the experiment with that name, created on first use. The experiment page compares it case by case with the other variants and marks it as run in Mira. Publishing the same label again replaces the variant.

## In the UI

**Quality → Evals** opens on an overview of every agent with a dataset: its pass rate, the trend over recent runs, its live quality from observers over the last 7 days, open findings and the last activity. Tabs hold datasets, experiments, observers, findings from session detectors and classifiers (similar ones grouped, each opening a page with its evidence and the Inspector's account), and the graders a case can use. **Run experiment** starts one from any dataset. Each agent's page has an **Evals** tab with the same view for that agent, plus the observers that watch it and the Eval Builder's proposals. On an observer's edit page, the **Answers**, **Conversations** and **Tools** tabs hold the graders of each scope.

## SWE-bench Lite

Beyond user-facing behavioral evals, Everruns ships a **SWE-bench Lite** harness for measuring coding-agent performance against the standard benchmark, using the same eval machinery (`post` verification messages run the test scripts that determine pass/fail).

## Related

- [Harnesses](/features/harnesses/), what an eval target runs
- [Change History](/features/change-history/), every change to an agent and how to restore an earlier configuration
