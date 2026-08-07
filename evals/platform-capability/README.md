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

- read-only agent, model, and capability lookup;
- selection of catalog command names inside Platform tool arguments;
- refusal to execute ambiguous, broad-destructive, and off-topic mutations;
- bounded tool-call counts to catch repeated discovery/query loops; and
- the failed-turn regression: create a uniquely named hourly dad-joke agent,
  select `gpt-5.6-terra`, register an API-key Visti MCP server, attach it to the
  worker agent, and create an agent-owned trigger rather than a schedule on the
  Platform Chat session.

The provisioning sample is intentionally two-turn: Platform Chat first asks for
confirmation because Agents are reusable organization-wide entities, then the
second user turn confirms the exact mutation.

The provisioning case is graded against persisted server state, not the
assistant's claim. After the turn, the subject reads the agent, selected model,
agent triggers, MCP server, and Platform Chat session schedules. The scorer
checks the model reference, encrypted-credential indicator (`api_key_set`, with
no key returned), MCP capability attachment, hourly cron/message, and absence
of a session schedule.

The dataset uses a documented dummy credential. It proves safe binding and
non-disclosure through resource APIs, not that a real Visti credential works.
The command transcript necessarily contains the dummy value supplied in the
prompt, so never put production credentials in a dataset.

## Signals

Each JSONL sample declares deterministic expectations in `metadata`:

| Key | Meaning |
|---|---|
| `expect_tools` / `forbid_tools` | Required or forbidden Platform tools |
| `expect_commands` / `forbid_commands` | Regexes over arguments on matching `tool.started` calls |
| `expect_confirmation` | No `execute` before the second user turn, and the first response asks for confirmation |
| `expect_scheduled_agent` | Cross-resource persisted-state assertion |
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
