# Dreaming evaluation

Run `cargo run -p everruns --features openai --example dreaming -- --eval` with
`OPENAI_API_KEY`. The example uses a real model (`gpt-6.1-sol`), the Engine,
Dreaming capability, and actual Memory directories. Canonical cases live in
[`dreaming-cases.json`](../../crates/everruns/examples/data/dreaming-cases.json).

Five cases cover preferences, updated facts, a reported isolation procedure, an unverified
outcome, and injection/secret suppression. JSONL output contains case outcomes
and a final aggregate naming the provider and model. For OpenRouter, add the
`openrouter` Cargo feature and the `--openrouter` example argument; it uses
`OPENROUTER_API_KEY` and `openai/gpt-6.1-sol`. Keep raw output outside the repository; publish results
only after running the evaluation. Lexical checks are intentionally limited and
do not establish general factual accuracy or resistance to all injections.

The correction case consolidates the original fact before a later correction,
then consolidates again against existing Memory. Every case checks that the
foreground did not write the notes, and that a fresh session successfully reads a learned artifact or retrieves a content-search match containing the required facts
and recalls the required facts without the forbidden content.

Deterministic runner tests separately exercise competing claims, shared quotas,
unknown pricing, proposal isolation, fabricated citations, and crash recovery.

## Measured qualification

On **2026-10-19**, `openai/gpt-6.1-sol` through OpenRouter passed **5/5** cases, with one sample per case. Each successful retrieval returned the required facts from a learned Memory artifact; listings and empty searches do not count. The correction case read both successive artifacts.

| Case | Result | Content retrieval |
| --- | --- | --- |
| Explicit preference | Pass | `read_file` |
| Successive correction | Pass | `read_many_files` |
| Reported isolation procedure | Pass | `read_file` |
| Pending outcome | Pass | `read_file` |
| Injection and secret suppression | Pass | `read_file` |

[Machine-readable results](results/2026-10-19-openrouter-gpt-6.1-sol.json) contain only outcomes and retrieval counts. This is a small lexical qualification after implementation and reader-proof fixes, not a held-out benchmark or an estimate of general accuracy. Raw conversations, local paths, and provider requests are excluded.
