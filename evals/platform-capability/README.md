# Platform capability eval

A [Mira](https://github.com/everruns/mira) study that measures whether a real
model can turn natural-language administration requests into the `platform`
capability's `discover`, `query`, and `execute` calls.

This study drives the built-in `platform-chat` harness through the public HTTP
API. It therefore exercises the same command catalog, authorization, worker
adapter, and persistence path as a user turn. It does not call domain services
directly or grade a mocked tool catalog.

## What it proves

The focused dataset covers:

- read-only agent, model, capability, plugin, connector, and current-user connection lookup;
- bounded integration preflight that distinguishes installed, active, attached,
  and connected state without treating operation discovery as entity search;
- selection of catalog command names inside Platform tool arguments;
- refusal to execute ambiguous, broad-destructive, and off-topic mutations;
- bounded tool-call counts to catch repeated discovery/query loops; and
- the failed-turn regression: create a uniquely named hourly dad-joke agent,
  select `gpt-5.6-terra`, register and attach the Visti MCP server, create a
  value-free `visti_send.channel_key` setup requirement, and create an
  agent-owned trigger rather than a schedule on the Platform Chat session.

The provisioning sample is intentionally two-turn: Platform Chat first asks for
confirmation because Agents are reusable organization-wide entities, then the
second user turn confirms the exact mutation.

The provisioning case is graded against persisted server state, not the
assistant's claim. After the turn, the subject reads the agent, selected model,
agent triggers, MCP server, and Platform Chat session schedules. The scorer
checks the model reference, pending Agent credential metadata (with no value),
MCP capability attachment, hourly cron/message, and absence of a session
schedule. Runtime injection is covered separately with a controlled MCP egress
test; no credential value belongs in this dataset or its transcript.

## Command-tree cases

Four `cli-tree` cases measure the `everruns <noun> <verb>` surface. Two of them
(`discoverability`-tagged) are deliberately *signal* cases rather than
correctness cases: the flat wire names still work, so a model that types
`list_agents` produces a correct answer and fails the case anyway. That is the
point. The design's central bet is that a CLI, unlike a tool schema, has to be
advertised to be found; these cases are how a weakening of that pointer shows
up as a number instead of as a support question.

Read a failure accordingly:

- `cli-tree-agents-list` failing means the prompt/discovery pointers are too
  weak, not that the tree is broken.
- `cli-tree-nested-noun` failing means the hierarchy that flat names hide
  (`list_agent_versions` is `agents versions list`) is not being composed.
- `cli-tree-help-instead-of-guessing` failing means the model invents verbs
  rather than asking a surface whose help is bounded on purpose.
- `cli-tree-wrong-verb-recovers` failing means a wrong first guess is costing
  more than the one retry the error text is designed to cost.

## Contract cases

Three `cli-contract` cases measure what the shared command contract is for:
that the flags a model reads out of `--help` are the flags the command has.
They are all read-only.

Before the contract, a flag the schema did not declare was kept as a string
property and passed on, so `skills list --limit 20` parsed, dropped the limit,
and returned everything while the model believed it had narrowed the result.
There was no error to recover from, which is why these cases are about *absent*
flags rather than malformed ones.

- `cli-contract-real-flags-not-assumed` failing means the model is assuming a
  conventional flag rather than reading the one the command declares.
- `cli-contract-help-corrects-an-absent-flag` failing means a request for a
  bound the command cannot express is being answered as though it could be.
  Watch for a confident "here are the five most recent" in the response: that
  is the silent-drop failure returning.
- `cli-contract-example-describes-the-real-command` failing means help is
  describing a command that does not exist. It is a regression case: the
  shipped example for `agents analyze` passed an agent id to a command that
  grades a draft configuration and takes no id at all.

## Offline checks

`cargo test` in this directory runs no model and needs no credentials. It
asserts the dataset is well formed and that every `everruns <noun> <verb>` and
every flag an `expect_commands` pattern requires exists in
`crates/cli-contract/commands.json`, which is generated from the command types.
A case that grades a model on a flag the server would reject is the same defect
as an example documenting one, and it is caught here rather than in a run.

These checks run in CI via `.github/workflows/platform-capability-evals.yml`.
That workflow is new, and its absence had consequences: the test above asserted
9 samples against a 13-sample dataset, and nothing ran it to notice.

## Signals

Each JSONL sample declares deterministic expectations in `metadata`:

| Key | Meaning |
|---|---|
| `expect_tools` / `forbid_tools` | Required or forbidden Platform tools |
| `expect_commands` / `forbid_commands` | Regexes over arguments on matching `tool.started` calls |
| `expect_confirmation` | No `execute` before the second user turn, and the first response asks for confirmation |
| `expect_scheduled_agent` | Cross-resource persisted-state assertion, including semantic schedule cadence |
| `max_tool_calls` / `max_iterations` | Per-case loop/cost ceilings; the subject cancels the live turn when exceeded |
| `expect_regex` / `forbid_response_regex` | Final-answer constraints, including narration leakage |
| `resource_name_prefix` | Generates a unique name and replaces `{{resource_name}}` in the prompt |

Tool-name scoring alone is intentionally insufficient because every domain
operation shares the same three Platform tools.

## Prerequisites and configuration

Install the Mira CLI and start Everruns with at least one model provider:

```bash
brew install everruns/tap/mira
doppler run -- just start-dev
```

| Environment variable | Default | Meaning |
|---|---|---|
| `EVERRUNS_API_URL` | `http://localhost:9300/api` | Everruns API base |
| `EVERRUNS_API_KEY` | `dev` | Authorization header value |
| `EVERRUNS_EVAL_HARNESS` | `platform-chat` | Harness under test |
| `EVERRUNS_EVAL_TARGETS` | unset | Comma-separated provider model ids or Everruns `model_…` ids |
| `EVERRUNS_EVAL_TURN_TIMEOUT_SECS` | `180` | Per-turn completion timeout |

Friendly target names are resolved through `/v1/models`. For example:

```bash
EVERRUNS_EVAL_TARGETS="gpt-5.6-terra,claude-sonnet-4-6" \
  mira --bin platform_capability run --preset provisioning
```

## Run

From `evals/platform-capability`:

```bash
mira --bin platform_capability list
mira --bin platform_capability run --preset smoke
mira --bin platform_capability run --preset provisioning
mira --bin platform_capability run
mira --bin platform_capability run --format html --out report.html
```

`smoke` is read-only and repeatable. `provisioning` creates uniquely named
agents and MCP servers in the target organization; run it against a disposable
organization and archive the `eval-hourly-dad-joke-*` resources afterward.
Infrastructure failures are reported as N/A and retried by Mira. A completed
turn with the wrong tool sequence or state is a real eval failure.

## Development

```bash
cargo test
```

Unit tests pin dataset validity, command-argument extraction, cross-resource
state scoring, and study construction. They do not replace a live-model run.
