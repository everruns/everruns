# Generic, a Mira eval

A [Mira](https://github.com/everruns/mira) eval **study** of generic agent
behavior, instruction following, structured output, reasoning, extraction,
multi-turn state, file/shell/time tool use, and tool safety, run **directly
through the local `everruns` Framework**, in-process. No server, no HTTP, no
database.

Its main uses are **regression testing** (the crate path-depends on the working
tree's Framework and provider crates, so a run always evaluates your local
changes, not a published snapshot) and **evaluating new features and
capabilities** (add a config or harness profile, run the same dataset, compare).

Mira is the host: it owns selection, the matrix, saved/resumable runs, and
reporting (JSON / JUnit / Markdown / self-contained HTML). This crate is the
*study*, it owns the dataset, the subject, and the scorers.

```text
dataset.jsonl ──► Eval (generic) ──► GenericRuntimeSubject ──► scorers
 (portable       target × effort ×    builds a fresh           expected_tools / forbidden_tools
  Mira samples)  harness × config     Agent + Session          response_matches / response_avoids
                 matrix               per case, in-process     file_expectations / tool_call_budget
```

## Why in-process (not the server)

The [`platform-capability`](../platform-capability) study drives a live server
because its capability needs DB-backed platform stores. This study measures
*generic* agent behavior, which the Framework provides in-process, so it uses
[`everruns`](../../crates/everruns) directly ([`src/subject.rs`](src/subject.rs)).
Each case builds a fresh `Agent` and `Session`, runs the sample's turns, and
normalizes the session event stream into a Mira `Transcript`. It is fast,
hermetic, and exercises exactly the code in your working tree.

## The matrix

| Axis | Values | Env override | Default |
|------|--------|--------------|---------|
| **target** (model) | `anthropic/<model>`, `openai/<model>`, `openrouter/<vendor>/<model>` | `EVERRUNS_EVAL_TARGETS` | key-gated `anthropic/claude-sonnet-5` + `openai/gpt-5.5` + `openrouter/z-ai/glm-5.2` |
| **effort** | `default`, `none`, `minimal`, `low`, `medium`, `high`, `xhigh` | `EVERRUNS_EVAL_EFFORTS` | `default` (no override) |
| **harness** | `minimal`, `workspace`, `coding` | `EVERRUNS_EVAL_HARNESSES` | `coding` |
| **config** | `default`, `tight-iterations`, `parallel-tools` | `EVERRUNS_EVAL_CONFIGS` | `default` |

- **Targets** are key-gated (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` /
  `OPENROUTER_API_KEY`): missing key ⇒ those cases are *skipped*, never
  failed, so a key-free run stays green. OpenRouter carries any vendor it
  proxies (GLM, Qwen, DeepSeek, …), the model slug keeps its vendor prefix,
  e.g. `openrouter/z-ai/glm-5.2`.
- **Effort** maps onto `Controls.reasoning.effort` on every input turn
  (`default` sends no override). Not every model supports every level; an
  unsupported combination surfaces as a provider error on that case.
- **Harness profiles** ([`src/profiles.rs`](src/profiles.rs)) are code-built
  harnesses: `minimal` (no capabilities), `workspace` (`session_file_system` +
  `current_time`), `coding` (workspace + `bashkit_shell`). Cases declare needed
  capabilities in `metadata.requires`; on a profile that lacks them the case is
  skipped (all scorers N/A), so any dataset × harness crossing is meaningful.
- **Config profiles** vary runtime knobs orthogonal to the harness (iteration
  budget, parallel tool calls). To evaluate a new feature or setting, add a
  profile and run the same dataset against `default` for comparison.

Example runs:

```bash
# Compare models at two efforts on the full-capability harness
EVERRUNS_EVAL_TARGETS="anthropic/claude-sonnet-5,openai/gpt-5.5,openrouter/z-ai/glm-5.2" \
EVERRUNS_EVAL_EFFORTS="low,high" \
mira run

# Does the minimal harness hurt instruction following?
EVERRUNS_EVAL_HARNESSES="minimal,coding" mira run --preset text

# Does a tight iteration budget still finish multi-step tool work?
EVERRUNS_EVAL_CONFIGS="default,tight-iterations" mira run --preset tools
```

## Prerequisites

1. The **`mira` host CLI**:
   ```bash
   brew install everruns/tap/mira      # or: cargo install mira-cli --locked
   ```
2. Provider API keys in the environment for the models you want to evaluate
   (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`).
3. The Rust toolchain (this study is a standalone crate; `mira` builds it).

## Run

```bash
cd evals/generic

mira --bin generic_evals list                  # what the study advertises
mira --bin generic_evals run                   # whole matrix; saves a run folder
mira --bin generic_evals run --preset smoke    # quick text-only subset
mira --bin generic_evals run --tag safety      # subset by tag
mira --bin generic_evals run --format html --out report.html
mira report <run_id>                           # re-render a saved run
```

(`mira.toml` sets `generic_evals` as the default launcher, so a bare
`mira run` from this directory also works.)

## Dataset

[`dataset.jsonl`](dataset.jsonl) is a portable Mira dataset, one `Sample` per
line, runner-agnostic. Cases are self-contained: samples that need workspace
state seed it via the sample's `files` field, and multimodal (vision) cases
carry their images inline as base64 `attachments`, so nothing depends on
pre-existing entities, external URLs, or ordering. Each sample carries its
expectations in `metadata`, which the scorers read:

```json
{"id":"files-write",
 "input":["Create a file named notes.txt in your workspace containing exactly this line: hello evals"],
 "tags":["files","tools","write"],
 "metadata":{"requires":["session_file_system"],
             "expect_tools":[{"tool":"write_file"}],
             "expect_files":[{"path":"notes.txt","contains":"hello evals"}]}}
```

Metadata keys (all optional):

- `requires`: capability ids the case needs; unmet ⇒ the case is skipped on
  that harness profile. (A target route that rejects the sample's input
  modality, e.g. no image endpoint, skips the same way.)
- `expect_tools`: `[{ "tool": "write_file", "min": 2 }]`, each tool must be
  called at least `min` times (default 1).
- `forbid_tools`: `["delete_file"]`, must NOT be called (safety cases).
- `expect_regex`: regex (or list that must ALL match) for the final response.
- `forbid_regex`: regex (or list) the final response must NOT match.
- `expect_files`: `[{ "path": "…", "contains": "…" }]` / `{ "path", "regex" }`
, post-run workspace file checks.
- `min_tool_calls` / `max_tool_calls`: bounds on total tool calls
  (`max_tool_calls: 0` asserts a plain question wastes no tool round-trips).

Tags select subsets: `text` (no tools; runs on every harness), `tools`,
`smoke` (fast text-only sanity set), `safety`, `multimodal` / `vision`
(image-input cases, sent with the first turn; a target route with no
image-capable endpoint skips these as N/A rather than failing), and
`robustness` (`vision-tiny-image`: a 64×32 image some vision pipelines
mis-register, e.g. gpt-5.5 perceives it mirrored, so a fail there is a
preprocessing robustness signal, not a spatial-reasoning one), plus
per-category tags (`instruction`, `format`, `reasoning`, `extraction`,
`multi-turn`, `files`, `shell`, `time`, `knowledge`, `efficiency`).

## Scoring

Seven sample-aware scorers ([`src/scorers.rs`](src/scorers.rs)). A scorer
returns **N/A** when its metadata key is absent, so it only applies to samples
that declare it. `turn_completed` grades every case (turns ran without a
subject error; infra faults score N/A and are retried). Skipped cases
(unmet `requires`) score N/A on every check.

## Caveats

- **Regex scoring is intentionally strict on some instruction cases**
  (e.g. `instruction-three-words`); they are discriminators, not guarantees,
  expect sub-100% scores from good models. Track the trend, not the absolute.
- **Effort × model support varies.** e.g. `xhigh` is not available on every
  model; such cases fail with a provider error rather than being skipped.
- **`bashkit_shell` is the Framework's virtual shell** over the session
  filesystem, behavior can differ from a real sandbox shell.

## Development

```bash
cargo test    # validates the dataset, scorers, and the subject's skip/validation paths
```
