# tool_search reveal eval

A [Mira](https://github.com/everruns/mira) study of the two ways the generic
`tool_search` capability makes a found tool callable
([`crates/core/src/builtins/tool_search.rs`](../../crates/core/src/builtins/tool_search.rs),
`Reveal`):

| Arm | What the model sees after `tool_search` finds a tool |
|---|---|
| `redefine` (default) | from the next step, the tool's definition in the tool list carries its full parameter schema |
| `in_result` (experimental) | the tool list keeps the permissive stub (`{"type":"object","additionalProperties":true}`) for good; the schema lives only in the search result |

`in_result` never changes the tool list, so the prompt cache stays warm on
models that cannot send tool changes inline. The cost it may carry is call
quality: structured tool calling has no real schema to emit against. The
engine still validates each call against the tool's registered schema before
it runs, so a bad call comes back as an `invalid_tool_arguments` error the
model can recover from. This study measures that cost before the default
changes.

## What runs

Each case builds an in-process Framework agent with the `tool_search`
capability (`{"reveal": <arm>, "threshold": 1}`, so every fake tool is
deferred) over 26 fake tools in five domains ([`src/registry.rs`](src/registry.rs)):
CRM, billing, calendar, tickets and inventory. The schemas are realistic and
strict: required fields, enums a person would not guess (`fra2` for
Frankfurt, `p2`), nested objects, arrays of objects, integer ranges, money in
cents, `additionalProperties: false` throughout, and neighbours with similar
names (`create_invoice` / `create_quote`, `create_event` /
`create_recurring_event`, `find_contact` / `find_company`).

Every fake tool validates its arguments against its own schema with
`jsonschema` and logs the call, as a cross-check. In practice the engine
refuses a call that does not match the registered schema before the tool
runs, so scorers grade every attempt from the `tool.started` and
`tool.completed` events, recording whether the engine or the tool refused it,
not the model's account of what it did.

Each case runs once per arm: `reveal` is a Mira axis, so a case key reads
`tool_search_reveal/<case>@<target>[reveal=in_result]`.

## Cases

| Case | What it measures |
|---|---|
| `tsr-invoice-line-items` | an array of line-item objects, prices converted to cents, enum currency; not the quote tool |
| `tsr-refund-enum` | an enum reason the request only paraphrases (`duplicate`), cents, a required boolean |
| `tsr-ticket-nested-customer` | a nested customer object with an enum plan, lowercase priority enum, labels array |
| `tsr-recurring-event` | the recurring tool, not the one-off one; nested recurrence, weekday codes, field names only its schema has (`first_start`, `occurrences`) |
| `tsr-event-attendees-location` | an array of attendee objects with a required boolean, nested location, integer array |
| `tsr-chain-contact-activity` | look up a contact id, then log an activity in a nested object |
| `tsr-chain-reserve-stock` | look up a SKU, then reserve stock with a warehouse enum named only as a city |
| `tsr-chain-escalate` | search tickets (open, not the solved duplicate), then escalate with enum team and integer level |

## Scores

| Scorer | Meaning |
|---|---|
| `correct_call` | each expected tool received a call that validated and matched the expected values (recursive subset; arrays order-free); no forbidden tool ran |
| `first_try` | the first call to each expected tool already validated |
| `rejected_calls` | informational: calls the tools refused (`reveal.rejected_calls` metric) |
| `calls_to_stub_before_search` | informational: calls to a tool before any `tool_search` result returned it (`reveal.calls_to_stub_before_search`) |

Token usage, iterations, tool calls and searches go into the tables too.
After `--run` the binary prints a per-target-and-arm table and a
per-case-and-arm table; `--runs-report <path>` (or
`EVERRUNS_EVAL_RUNS_REPORT`) writes one JSON line per run with the full call
log, for reading failures.

## Run

```bash
cargo build
export EVERRUNS_EVAL_TARGETS=openrouter/openai/gpt-6-luna,openrouter/mistralai/mistral-large-4-0,openrouter/meta/muse-spark-1.3-contributor
export EVERRUNS_EVAL_TRIALS=3
doppler run -- ./target/debug/tool_search_reveal --run --runs-report runs.jsonl
mira --bin tool_search_reveal run --preset smoke
```

Targets are `anthropic/<model>`, `openai/<model>` or
`openrouter/<vendor>/<model>`; unset, one OpenRouter model runs. A missing key
is an infra error (N/A), not a failure, and infra failures are left out of the
rates. `--run` runs cases concurrently (`EVERRUNS_EVAL_CONCURRENCY`, default
8) through `mira::runner::run_case`, so scoring matches the Mira host's.
`--filter <substring>` and `--sample <glob,...>` narrow a run, e.g.
`--filter reveal=in_result`.

## Offline checks

`cargo test` runs no model and needs no key. It checks that every expected
input in the dataset validates against its tool's own schema (a case the tool
would refuse is a broken case), the matcher, and drives one scripted run per
arm with a simulated model behind a driver that records each request's tool
list: search, a refused call, then the right call. It asserts that `in_result`
sends the stub for the found tool on every request and `redefine` sends the
full schema after the search, and that the engine refused the bad call in
both, before it reached the tool. The simulated model only proves the wiring; the study
itself always runs real models.

## Results

First run, 2026-10-13: 3 trials of 8 cases per arm per model, through
OpenRouter. Rejected calls, tool calls and tokens are means per run.

| Model | Arm | `correct_call` | `first_try` | Rejected | Tool calls | Tokens |
|---|---|---|---|---|---|---|
| `openai/gpt-6-luna` | redefine | 24/24 | 24/24 | 0 | 2.42 | 4,991 |
| `openai/gpt-6-luna` | in_result | 24/24 | 24/24 | 0 | 2.38 | 5,006 |
| `mistralai/mistral-large-4-0` | redefine | 24/24 | 17/24 | 0.33 | 2.88 | 18,123 |
| `mistralai/mistral-large-4-0` | in_result | 24/24 | 21/24 | 0.12 | 2.62 | 11,790 |
| `meta/muse-spark-1.3-contributor` | redefine | 24/24 | 24/24 | 0 | 2.54 | 43,689 |
| `meta/muse-spark-1.3-contributor` | in_result | 0/24 | 0/24 | 4.83 | 9.50 | 230,279 |

GPT-6 Luna and Mistral Large 4 lose nothing with `in_result`; Mistral's
rejections in both arms are calls made before searching, fixed after one
engine error. With only the stub to go on, Muse Spark sends every number,
boolean and object as a JSON-encoded string (`"amount_cents": "2550"`,
`"customer": "{\"email\": ...}"`), repeats the call until the iteration cap,
and then reports success it did not have. `in_result` stays opt-in.

