# Evaluate an Everruns agent

Two tools, for two questions:

- **Evals** answer "does it work?" before a change ships. An eval is a set of
  cases; each case runs a real, fresh session against the agent and scores the
  result from 0.0 to 1.0. A run executes every case against one target (agent,
  harness, model), so two runs compare two configurations.
- **Observers** answer "is it still working?" on real traffic. An observer
  samples completed turns of production sessions and scores them with rules or
  an LLM judge.

Commands below are the `everruns` command language (see the [cli.md](cli.md)).
Creating evals, runs and observers are changes: use MCP `execute` or the CLI.
Runs and judges spend model credits; say roughly how many sessions a run
starts before you start it.

## 1. Collect cases from reality

Good cases come from sessions the agent already had, not from imagination:

- `everruns sessions list --help`: find recent sessions for the agent (filter
  by agent), including ones the user said went badly.
- `everruns sessions messages list --help`: read the user's message and the
  reply to turn each into a case.
- Cover the common request, the edge case that failed, and one request the
  agent should refuse or hand off. Five to ten cases is a useful start.

## 2. Write the eval

```bash
everruns evals create --help          # name, description, target, tags
everruns evals cases create --help    # one case per test
```

A case has `conversation` (the user messages, sent in order), `scorers`, and
optional `max_turns`, `timeout_seconds`, `post` messages (run after the agent
goes idle, for example a test script) and `artifacts` (session files to keep).

Pick scorers that check what the user cares about, cheapest first:

| Scorer `type` | Passes when |
|---|---|
| `contains`, `not_contains`, `regex` | The final reply has (or lacks) the text |
| `tool_called`, `tool_not_called`, `tool_call_count` | The agent used the right tools, the right number of times |
| `turns_within` | It finished within N turns |
| `file_contains` | A session file contains the text |
| `json_schema` | The reply is JSON matching a schema |
| `citation_faithful`, `citation_judged` | Its citations support its claims |

Each scorer has an optional `weight`; the case score is the weighted average.
Prefer checks on behavior (tools, files, structure) over exact wording.

## 3. Run and compare

```bash
everruns evals runs create --help     # target or model_override per run
everruns evals runs get --help        # status, per-case results and scores
everruns evals runs list --help
```

- **Model choice**: run the same eval twice with different `model_override`
  values and compare the scores per case.
- **Prompt change**: run before and after the change (see [build-agent.md](build-agent.md)).
- Every case is a real session: open a failing one and read it like any other
  ([debug-session.md](debug-session.md)).

Report a table: case, score before, score after (or model A, model B), and one
line on what changed. `everruns evals runs share --help` mints a read-only link
to send to people outside the organization; ask before creating one.

## 4. Watch live traffic

```bash
everruns observers create --help      # match_config, sampling_rate, scorers
everruns observers scores list --help
```

An observer scorer has a `key` (the score series name) and either
`method: "rule"` with one of the eval scorers above (except `file_contains`),
or `method: "llm_judge"` with a `rubric` describing what high and low scores
mean. `sampling_rate` defaults to 0.1; judge calls bill the organization, so
keep it low on busy agents. Eval sessions are never scored by observers.

## When a case fails

1. Open the case's session and find the cause ([debug-session.md](debug-session.md)).
2. Change the agent ([build-agent.md](build-agent.md)), with `--reason` naming the case.
3. Run the eval again and report before and after.

A failure from a real session that the agent now handles belongs in the eval
for good, so the next change cannot bring it back.
