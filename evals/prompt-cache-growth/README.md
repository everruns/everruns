# Prompt cache growth eval

A [Mira](https://github.com/everruns/mira) study of how well a hosted session
keeps the provider's prompt cache as its history grows.

A healthy session only appends to its previous request. From the second
model call on, most of each prompt is then a cache read, and the cached share
holds or rises as the conversation gets longer. When something earlier in the
request changes, the provider re-reads everything after it at the full input
price. On a long session that is most of the bill: in October 2026 one
scheduled Platform Chat session re-sent a ~300k-token history uncached every
five minutes because each tool result's time fact was re-rendered on every
request.

## What runs

The subject drives a running Everruns server over HTTP
([`src/subject.rs`](src/subject.rs)). For each case it creates a Platform Chat
session, plays the turns in order, then reads the session's `llm.generation`
events. It uses the server, not an in-process Framework agent, because the
hosted path (the worker loading history from the server) is where the cache
breaks came from.

For each model call, [`src/cache.rs`](src/cache.rs) reads the uncached input,
cache reads and cache writes. A call that reads less than half of the previous
call's prompt from cache is a **break**. The study diffs the two stored request
bodies in the order the provider reads them, and the first part that changed
is the cause:

| Cause | Meaning | Defect |
|---|---|---|
| `model` | another model; caches never span models | no |
| `cache_key` | the provider's cache routing key changed | yes |
| `instructions` | system prompt or instructions changed | yes |
| `tools_reordered` | the tool list changed before its end | yes |
| `tools_appended` | tools were only added at the end; on models that receive the whole tool list before the conversation, this still costs the history | no |
| `history_rewritten` | an earlier message changed (a re-rendered fact, a masked tool output) | yes |
| `provider` | the request only appended; the provider missed anyway (eviction, routing) | no |
| `unknown` | a request body was not captured | no |

## Cases

| Case | What it grows |
|---|---|
| `growth-chat` | seven plain turns, no tools |
| `growth-tool-results` | a read-only platform lookup on every turn, so every request carries tool results and their facts blocks |
| `growth-tool-reveal` | lookups that need tools found by tool search, so the tool list grows mid-session |
| `growth-long` | twelve mixed turns |

All cases are read-only against the organization.

## Scores

| Scorer | Meaning |
|---|---|
| `succeeded` | every turn completed |
| `cache_hit_rate` | share of prompt tokens read from cache after the first call, at least the case's `min_hit_rate` |
| `prefix_stable` | no break with a defect cause |

Metrics: `cache.hit_rate`, `cache.hit_rate_late` (the later half of the
calls, which shows whether the cache holds as the session grows),
`cache.breaks`, and `cache.breaks.<cause>`, plus the shared token and cost
metrics from [`../common`](../common/README.md). The transcript metadata
keeps the per-call table (`cache.calls`) and one line per break
(`cache.breaks_detail`).

## Run

Start a stack from the tree you want to measure (see the repository
`AGENTS.md`), then:

```bash
cargo build
export EVERRUNS_API_URL=http://localhost:27100/api
./target/debug/prompt_cache_growth --run
./target/debug/prompt_cache_growth --run --filter growth-chat
mira --bin prompt_cache_growth run --preset smoke
```

`EVERRUNS_EVAL_TARGETS` takes server models (a `model_…` id, `model_id` or
display name), comma-separated. The default pair,
`meta/muse-spark-1.3-contributor,gpt-6-luna`, covers both ways tool changes
reach a model: Muse through OpenRouter receives the whole tool list, and GPT-6
Luna receives tool changes inline. `--run` plays cases one at a time, because
concurrent sessions share the provider's cache and would blur each other's
rates. `EVERRUNS_API_KEY` (default `dev`) and `EVERRUNS_EVAL_TURN_TIMEOUT_SECS`
(default 300) apply as in the platform study.

## Development

```bash
cargo test
```

The tests cover cause attribution and the scorers on recorded event shapes. No
model or server is needed.
