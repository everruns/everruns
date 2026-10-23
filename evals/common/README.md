# Everruns event usage

`everruns_usage.rs` is shared by the Rust/Mira subjects that consume Everruns
session events. It counts one usage record per reasoning iteration, preferring
`llm.generation` (including folded compaction cost), then `reason.completed`,
then message completion. Turn/session rollups and arbitrary tool-result JSON
are excluded. A recursive walk over these events counts the same usage several
times and misses Everruns' actual/estimated cost fields.

These studies use Everruns' **disjoint** convention: `input_tokens` is uncached
input; cache-read and cache-creation are separate buckets. `llm.input_tokens_total`
adds those buckets across calls. Mira's generic `total_tokens()` does not add
these cache buckets; use the explicit study metric when comparing context.
Output tokens accumulate separately. Neither accumulated input nor schema bytes
measure peak context; `llm.input_tokens_peak` is the largest observed request.

The projection records request and usage-record counts separately, so a request
without usage remains visible. First/peak input metrics require complete usage
coverage. Tool schema bytes come only from the captured provider request's
`tools` array, never the event's display summaries. They measure serialized
array bytes, not model tokens. Missing capture leaves footprint metrics absent;
inline tools carried in conversation items are not counted in that array.
Provider-reported cost coverage is explicit; effective cost prefers the stored
effective total, then actual, then estimated cost. This is not invoice
reconciliation or a causal prompt-cache measurement.

The platform study's in-process HTTP subject consumes each provider response
directly instead. It subtracts cached reads from OpenRouter's inclusive prompt
count exactly once and uses the same disjoint reporting convention.

Tests compile with each importing study. The MCP study also offers
`--reaccount-report <JSONL path or -> --output <JSON path>` to recompute compact
per-case usage/metrics from retained raw events without another model call.
