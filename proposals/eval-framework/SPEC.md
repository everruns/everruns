# Spec: a Rust-first, code-first evaluation framework

Status: **proposal / prototype**. This directory is a standalone workspace,
excluded from the everruns build. It is meant to be reviewed, refined, then
handed over to its own repository. The crate name `evals` is a placeholder.

## 1. Problem

We run agents and tools against datasets to measure quality, and we do it three
incompatible ways:

| Repo | Harness | Lang | Subject under test | Datasets | Scoring | Reporting |
|------|---------|------|--------------------|----------|---------|-----------|
| yolop | `bench/yoloeval` | Python | a whole agent **CLI** (yolop/claude-code/codex/pi) on SWE-bench | HF parquet + JSON suites | Docker `FAIL_TO_PASS` + rich metrics | per-run JSON + `summary.json`, `matrix.yaml` |
| bashkit | `bashkit-eval` crate | Rust | a single **`Tool`** in an agent loop (persistent VFS) | JSONL | string-keyed checks (`file_exists:…`) | JSON + markdown + `/benches` site |
| everruns | `llm-tests` + product eval subsystem | Rust | runtime **sessions/turns** | rstest `#[case]` | `Scorer` enum in `core` | assertions |

Consequences: bashkit reimplemented providers from scratch; everruns maintains a
second scorer vocabulary; yolop is Python-only. Meanwhile `everruns-runtime`
(published) already provides turn execution, a `DriverRegistry` over ~6
providers, an offline `llmsim`, in-memory/real-disk stores, and a canonical
JSONL `Event` transcript that yolop and coding-cli already emit.

**Goal:** one Rust-first framework — a *developer tool* shaped like a test
runner — that all three can adopt, with first-class code-based authoring,
selective runs, model matrices, and built-in visualization. Polyglot/other-
language targets are a supported secondary via a subprocess subject.

Non-goal: the everruns **product** eval subsystem (`crates/core/src/eval.rs`,
`specs/evals.md`) and online observers. Those are runtime features for end
users. This is a dev tool. The two should *share the scorer vocabulary* but are
otherwise distinct.

## 2. Prior art we are matching

- **Inspect AI** — `Task = dataset + solver + scorer`, model chosen at runtime,
  20+ providers, composable scorers (model-graded QA, F1, pass@k, bootstrap),
  the **Inspect View** transcript UI. The closest model; we follow it.
- **vitest-evals / Flue** — evals are *just tests* (`describeEval`), fresh agent
  per case, deterministic asserts + `toSatisfyJudge` in one file, `serve` UI +
  JSON + GitHub Action summary. Lesson: make evals feel like the test runner.
- **promptfoo** — YAML matrix + CI regression + PR comparison action.
- **braintrust** — hosted experiment/PR-diff UX; the visualization bar, minus
  the SaaS lock-in.

Consensus: code-first, runtime model selection, composable deterministic +
LLM-judge scorers, transcript-level visualization, CI-native output.

## 3. Core model

Three composable pieces, all implemented in the prototype:

```
Eval = Dataset(Sample…) + Subject + [Scorer…]  ×  model matrix
```

- **`Sample`** — one dataset row: input turns, optional `target`, seeded
  `files`, `tags`. Language-agnostic JSON; inline in Rust for small evals.
- **`Subject`** (trait) — the thing under evaluation. `async fn run(&Sample,
  &RunCx) -> Transcript`. One adapter per *shape*:
  - `RuntimeSubject` — drives an `everruns-runtime` session. **(prototype)**
  - `ToolSubject` — a bashkit `Tool` in a minimal agent loop. *(spec)*
  - `CliSubject` — an external binary; reads back the JSONL `Event` transcript
    or a git diff. The **polyglot / other-language** path (replaces yolop's
    Python agent adapters). *(spec)*
- **`Transcript`** — normalized run result: final response, iteration/tool
  counts, token+cost `Usage`, best-effort tool names, and the raw serialized
  `Event` stream. All subjects produce the same shape, so scorers and reporting
  are shared.
- **`Scorer`** (trait) — `async fn score(&Sample, &Transcript) -> Score`
  (`value` 0..1, `pass`, `reason`). Built-ins: `contains`, `not_contains`,
  `regex`, `tool_called`, `tool_calls_within`, `turns_within`, `succeeded`, the
  `scorer(name, closure)` escape hatch, and `model_graded(rubric, judge)`. The
  bar for bespoke logic is a closure, not a new enum variant — this generalizes
  both bashkit's string checks and everruns' `Scorer` enum.

### Matrix

`models` is a first-class axis on an `Eval`. The runner expands
`evals × models × samples` into independently-addressable cells
(`greet/hi@sim`). `ModelSpec::{sim, anthropic, openai, …}` build cells; missing
API keys **skip** rather than fail, so the default run is green offline. The
design extends to arbitrary axes (reasoning effort, harness variant) as a
cross-product; only the model axis is in the prototype.

### Selective evaluation

Mirrors `cargo test`: a substring `filter` on the case key and a `--tag`
narrow. In the macro design (below) this is delegated to the test harness.

## 4. Developer experience — the headline

Evals should feel like the unit-test runner the team already uses.

**Prototype (today):** an explicit builder + `Runner`.

```rust
Eval::new("greet")
    .case("hi", "Say hi and tell me the answer.")   // inline; no dataset file
    .subject(RuntimeSubject::new(runtime_factory()))
    .scorer(succeeded())
    .scorer(contains("42"))
    .scorer(model_graded("Is it responsive?", judge))
    .models([ModelSpec::sim(), ModelSpec::anthropic("claude-haiku-4-5")])
    .build()
```

**Target polish:** an `#[eval]` attribute that registers a function returning
`Eval` and runs it under a `libtest-mimic` harness (`[[test]] harness = false`),
so evals inherit `cargo test`-style discovery, `--list`, filtering, and
parallelism — and a `models = [...]` arg expands the matrix like `rstest`'s
`#[case]`:

```rust
#[eval(models = ["anthropic/claude-opus-4-8", "openai/gpt-5.5", "sim"])]
fn file_operations() -> Eval { /* same builder */ }
```

```bash
cargo eval                       # all evals, default matrix
cargo eval file_ops              # selective (substring), like cargo test
cargo eval --tag smoke           # selective by tag
cargo eval --models sim          # restrict the matrix
```

YAML/JSONL stays the *secondary* on-ramp (`Dataset::jsonl`, a future
`Eval::from_yaml`) — a thin loader over the same core, not a second engine.

## 5. Reporting & visualization

- **Canonical JSON record** — adopt yolop's proven schema (per-scorer results,
  tokens, cost, per-tool breakdown, timing, stop reason). The prototype emits a
  lean version (`report::to_json`).
- **CI-native**: `--format json|md|junit|tap`. JUnit/TAP surfaces evals in any
  CI test UI; markdown for PR job summaries.
- **Built-in viewer**: a self-contained single-file `report.html` (JSON
  embedded, small bundled JS) for transcript drill-down and a model×eval
  heatmap — the Inspect View / braintrust experience at zero infra. `eval report
  --open`. The prototype prints the matrix grid to the terminal as a stand-in.
- **History**: generalize bashkit's `/benches` aggregation to consume this
  record for trend lines.

## 6. Why build on `everruns-runtime`

It already provides what a harness needs and is published: `InProcessRuntime`
turn execution, `DriverRegistry` (Anthropic/OpenAI/Gemini/OpenRouter/Bedrock/
LlmSim), offline `llmsim` (key-free CI), pluggable in-memory/real-disk stores,
and the serialized `Event` transcript. `RuntimeSubject` is ~90 lines over it.
The prototype builds and runs against the **real** runtime (path deps); for
handover, swap to `everruns-runtime = "0.15"`.

## 7. Migration

- **everruns** — collapse the `llm-tests` matrix into evals; keep the product
  eval subsystem separate but factor `Scorer` into a small shared crate.
- **bashkit** — `bashkit-eval` becomes a thin `ToolSubject`; JSONL datasets load
  unchanged; delete the hand-rolled providers.
- **yolop** — SWE-bench logic becomes a `CliSubject` + a `ProcessScorer` around
  the Docker harness. The Python harness can run *as* a `CliSubject` during
  transition, proving the polyglot path immediately.

## 8. Prototype scope (this directory)

Implemented and runnable offline (`cargo run --example coding_eval`):
`Sample`/`Dataset`/`Transcript`/`Score`/`Usage`, the `Subject` and `Scorer`
traits, `RuntimeSubject` over real `everruns-runtime`, all deterministic scorers
+ `model_graded`, the model matrix with key-based skipping, `cargo test`-style
filter + tag selection, terminal matrix report + JSON export.

Deferred to implementation: the `#[eval]` proc-macro + `libtest-mimic` harness,
`ToolSubject` and `CliSubject`, the HTML viewer, JUnit/TAP, cost caps, and
arbitrary matrix axes. Each has a defined seam above.
