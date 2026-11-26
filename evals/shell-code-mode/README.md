# Code Mode comparison eval

This study compares `surface=native`, `surface=cli` (Bashkit Shell Code Mode),
and `surface=monty` on the same tasks, fixtures, model and trial count. The
binary and study retain the `shell_code_mode` name for existing launchers.
Monty keeps its real native-schema behavior; we do not hide tools artificially.
A surface scorer rejects direct task-tool fallback on the CLI and Monty arms.
Native discovery is permitted, and only completed calls under the correct
execution parent count as nested task evidence.

The original shell arm is a [Mira](https://github.com/everruns/mira) study of whether a model finds and
calls a tool it cannot see. With the `shell_code_mode` capability the agent's
tools leave its tool list and become one shell command, `tools`; the only ways
to find one are `tools --help`, a source's help, and `tools search`. The
capability pays off only if models find tools about as fast as they did with
direct tool schemas, and this study is how that gets measured rather than
assumed.

## What runs

The CLI arm builds an in-process Framework agent with `bashkit_shell` and
`shell_code_mode` over a small fake registry ([`src/registry.rs`](src/registry.rs)):
three MCP-style sources (`crm`, `tickets`, `calendar`) and two top-level tools,
plus the Framework's own built-in tools, which go behind `tools` too.
Everything between the model and the fake tools is the shipped code: the
capability's prompt addition, the hook that hides the registry and names the
sources on `bash`, the builtin's help, search, input parsing and error
envelopes, and the per-call policy the turn engine installs. Only the tools
are fake, so a run needs one model key and no server.

Each fake tool records the call it received; scorers grade actual effects from
that log and routing from nested tool events, not the model's claims.

## Cases

| Case | What it measures |
|---|---|
| `tis-find-by-keyword` | a top-level tool found from a plain-language need that names neither tool nor source |
| `tis-find-by-source` | a tool found through its source's listing, with input in the documented format |
| `tis-choose-between-similar` | `search-articles` chosen over `search-tickets` on the same source |
| `tis-json-input` | one JSON object with an enum and an array, passed right the first time |
| `tis-recover-wrong-name` | an old runbook names `get-contact`, which does not exist; discover and use `find-contact` |
| `tis-chain-two-tools` | one tool's output (a contact id) fed into another |
| `approval-allow` | reserve once, pause for approval, apply once and verify |
| `approval-reject` | reserve once, pause for rejection and never apply |

## Counting calls

Calls are counted the way the platform-capability study counts `everruns`
calls, with the same classifier, included by path from
[`../platform-capability/src/friction.rs`](../platform-capability/src/friction.rs):

| Class | Meaning here |
|---|---|
| `help` | every `tools` invocation only read: `tools --help`, `tools <source>`, `tools <tool> --help`, or `tools search …` |
| `rejected` | the builtin refused the call: `unknown_command` (no such tool or source) or `invalid_input` (the tool's schema refused the object) |
| `real` | a tool ran and nothing was refused; a tool's own `tool_error` is not a refusal of the surface |
| `other` | the call never invoked `tools` |

They land as transcript metrics (`friction.*`), as the informational
`tools_friction` score, as a per-case and overall table after `--run`, and,
with `--friction-report <path>` (or `EVERRUNS_EVAL_FRICTION_REPORT`), as one
JSON line per run with the commands tried, each refusal, and the help pages
read. The same tuning loop applies (the `tune-cli-help` agent skill), with the
`tools` help and tool descriptions in place of the command tree's.

## Run

```bash
export EVERRUNS_EVAL_TARGETS=openai/gpt-6.2-sol
export EVERRUNS_EVAL_TRIALS=3
doppler run --command './target/debug/shell_code_mode --run --friction-report friction.jsonl'
mira --bin shell_code_mode run --preset smoke
```

Targets are `anthropic/<model>`, `openai/<model>` or
`openrouter/<vendor>/<model>`; unset, one OpenRouter model runs. A missing key
is an infra error (N/A), not a failure.

## What the first runs should watch

- **The root help is long enough to be cut.** The bash tool's default `auto`
  output keeps the head and tail of a long success. With the Framework's
  built-in tools listed as top-level `tools` commands, `tools --help` here
  exceeds that window and the `Servers:` list in its middle is dropped from what
  the model reads (the offline test shows it). The `bash` description still
  names the sources, so this may cost nothing; if `tis-find-by-source` or the
  `crm` cases show extra help reads, this is the first suspect.
- **Rejections on `tis-recover-wrong-name` are expected, once.** More than one
  means the error text is not pointing at `tools crm --help` or `tools search`
  clearly enough.

## Offline checks

`cargo test` runs no model and needs no key. It checks the dataset names real
registry tools, the scorers' matching rules, and the classifier against the
builtin's real envelopes, and drives one scripted run through the whole stack
with a simulated model: help, search, a wrong name, a refused input, then the
right call, with the friction count and the call log both checked.
`.github/workflows/shell-code-mode-evals.yml` runs them in CI.

## Monty and approval qualification

The dataset also includes `approval-allow` and `approval-reject`. A reservation
is a counted effect before the gated apply operation. The evaluator answers the
real Engine approval request, checks that apply has not run before the decision,
and grades the recorded operation counts. Monty must start exactly one Python
program and use a saved continuation after the decision. Rerunning source,
duplicating the reservation, bypassing Python, or ignoring rejection fails.
These cases exercise a durable approval *pause* and a fresh interpreter worker;
they do not claim that an in-memory Engine survives host termination.

Install the pinned Monty runtime and launcher as described in the
[capability documentation](../../docs/capabilities/python-code-mode.md). Create
private journal storage outside agent workspaces, then run a bounded comparison:

```bash
export EVERRUNS_MONTY_BIN=/opt/everruns/monty/bin/everruns-monty-worker
export EVERRUNS_MONTY_STATE_DIR=/var/lib/everruns-evals/monty
# Pre-create that directory with mode 0700; use dedicated synthetic eval sessions.
export EVERRUNS_EVAL_TARGETS=openai/gpt-6.2-sol
export EVERRUNS_EVAL_TRIALS=3
cargo run --locked -- --run --sample tis-chain-two-tools,approval-allow,approval-reject \
  --runs-report results/code-mode.jsonl
```

Every Monty case checks the configured worker before spending model tokens.
Missing runtime, unsupported kernel isolation or private storage is an
infrastructure error, never a pass or an automatic weaker-runtime fallback.
An explicitly configured interpreter-only test launcher must be labeled using
`EVERRUNS_EVAL_ISOLATION_LABEL`; such evidence cannot qualify deployment isolation.

`--runs-report` preserves surface, placement, raw events, fixture call logs,
scores, usage (including cache buckets), provider-reported cost when available,
model calls, duration, Python source/resume counts and nested failures. Shell
help/rejection friction remains shell-specific; it is not a cross-language
quality score. Historical runs used the previous shell-specific recovery prompt;
new comparisons use the same interface-neutral request in every arm.

The ignored `monty_approval_eval_uses_the_engine_pause_and_saved_continuation`
test drives this exact evaluator with a scripted model and the real runtime:

```bash
cargo test --locked monty_approval_eval -- --ignored
```

The required Monty CI job runs it with the hardened worker. Ordinary study CI
runs deterministic scorer tests without a model key; it does not assert live
model quality.

## Hosted boundaries

`hosted_smoke.py` runs a separate workspace round-trip through a dedicated
Server or agentd eval agent. It creates a fresh session, requires one write and
a direct read-back of the expected artifact, then a `stat_file` call through the
selected surface. `read_file` and `write_file` deliberately stay direct in both
adapters. Assistant claims, repeated writes and direct fallback for `stat_file`
cannot pass. It records placement and session events, and never publishes to an
existing conversation.

Configure dedicated agents with the same model, `session_file_system`, and the
selected execution capability. Provision Monty on the *executing host*. Supply
`EVERRUNS_EVAL_API_KEY` when required, without putting credentials in arguments.
For Server use the appropriate platform session collection URL; agentd uses
`/v1/sessions`. For example:

```bash
python3 hosted_smoke.py --placement agentd --surface monty \
  --sessions-url http://127.0.0.1:7700/v1/sessions --agent monty-eval \
  --runtime-qualification 'Linux launcher qualification report reference' \
  --output results/agentd-monty.json
```

This is a hosted boundary smoke, separate from the Mira comparison. It does not
qualify hosted approval recovery, executor replacement or Raft leader movement.
Those need a running deployment and fault injection; absent results are recorded
as unqualified, not inferred from Framework results. Keep reports from dedicated
synthetic agents only. `python3 -m unittest test_hosted_smoke.py` checks that its
grader rejects false evidence.

## Reading results

Compare task success before token or latency means: an early stop can be cheap
because it did not finish the task. `llm.input_tokens_total` includes uncached,
cache-read and cache-creation input; provider cost comes from generation events.
Script counts, nested effects and model calls measure different things. Approval
cases additionally require the recorded decision to precede execution, exactly
one reservation, and a saved Monty continuation rather than restarted source.
A passing rejection case proves non-execution, not successful application.

Small one-trial matrices are integration evidence, not a ranking. Preserve failed
trials, provider failures, skipped targets and runtime qualification labels in the
report. Framework model results cannot qualify a hosted deployment's recovery or
kernel isolation.

Recorded synthetic traces and provenance are in
[`results/monty-2026-11-25/`](results/monty-2026-11-25/). The report retains initial
failures and labels corrected grading and fresh retests separately.
