# MCP scale study

This is a Mira study of actual Everruns Framework behavior. Its subject attaches Rust fixture servers with `McpServer::stdio`, then runs the current working-tree Framework and its canonical MCP discovery, bounded `tool_search`, and Tools in Shell implementation. The fixture speaks newline-delimited MCP JSON-RPC, paginates `tools/list`, dispatches `tools/call`, and persists isolated ticket and deployment state in per-trial fixture files across MCP process launches. No independent search algorithm or gateway stands between Framework and the fixture.

The study varies total catalog size (50, 600, 2,000, 10,000), server count (one or ten), and call surface (native, CLI, hybrid). Cases include an ordinary read, a server-description-only disambiguation, an explicitly read-only request, and a staged rollback that must be verified while production remains unchanged. Scores use Framework tool events and returned fixture state. A successful final response by itself does not pass.

## Build and inspect the fixture

From this directory:

```bash
source /workspace/.tools/activate.sh
cargo build
cargo run -- --offline-catalog
cargo run -- --offline-stress --output results/tool-search-index-stress.json
```

The standalone crate path-depends on the current `../../crates/everruns` and enables stdio MCP plus Bashkit. The child server uses the same compiled executable as the study subject.

The [study manifest](Cargo.toml) carries the checkout's exact Bashkit source
patches for its Wasmtime security fix. Cargo does not inherit a dependency's
workspace patches, so standalone Framework hosts enabling Bashkit need equivalent
patches in their own workspace. The adapter uses the stable analysis labels shared
by the registry and patched APIs.

## Run with a model

Set `OPENROUTER_API_KEY` in the environment. The default synthetic-data-only target is `openrouter/meta/muse-spark-1.3-contributor`; target labels do not contain credentials. From this directory:

```bash
EVERRUNS_EVAL_TARGETS=openrouter/meta/muse-spark-1.3-contributor \
EVERRUNS_EVAL_TRIALS=3 \
EVERRUNS_EVAL_CONCURRENCY=1 \
  cargo run --locked -- --run \
  --sample readonly-ticket,staged-rollback \
  --filter 'catalog_size=600,servers=1,'
```

`cargo run` builds the Rust executable and uses it for the child MCP servers. The explicit sample and delimiter-safe filter keep this run to 600 tools on one server. Use `catalog_size=2000,servers=1,` for the 2,000-tool arm, and `servers=10,` for ten-server cases. Leave the surface axis unfiltered to compare native, CLI, and hybrid. `--runs-report /tmp/mcp-scale.jsonl` writes full transcript events, score reasons, usage, and metrics as JSONL. Missing target keys are marked skipped; provider and service failures are reported separately as infrastructure failures. Neither counts as a pass.

The recorded runs used a managed cloud environment: `/workspace/.tools/activate.sh`, Doppler credentials, a shared `CARGO_TARGET_DIR=/workspace/everruns/target`, and `historical/python-prototype/cloud_run.py` to represent the configured proxy host by its resolved IP. That environment-only adapter preserves the proxy endpoint and TLS policy; it is unnecessary on ordinary hosts. The evaluator, fixture, discovery, scoring, and reports are Rust/Mira.

The full Mira matrix has many combinations. Use filters deliberately for paid runs. Catalog sizes 50 and 10,000 are available for focused checks; 10,000-tool discovery is a stress arm and should be run separately from the representative model comparison.

`--offline-stress` measures index construction and repeated retrieval through the exact `ToolSearchIndex` used by the working-tree Framework, for catalogs of 50, 600, 2,000, and 10,000 tools across one and ten server partitions. It reports p50/p95 timings, serialized catalog and top-10 hit bytes, and a server-description-only query. Timing is local synthetic microbenchmark evidence for the recorded build profile and machine; serialized byte counts are not heap-memory measurements and this does not measure model latency, MCP process startup, or cache refresh behavior.

The committed stress run used rustc 1.96.0, Linux x86_64, four logical CPUs, and the dev profile. Each row below shows catalog serialized bytes, index build p50/p95, and search p50/p95 for ticket, deployment, and server-description queries, in microseconds:

| Tools | Servers | Catalog bytes | Build p50/p95 | Ticket p50/p95 | Deployment p50/p95 | Description p50/p95 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 50 | 1 | 36,189 | 4,483 / 4,843 | 54 / 113 | 64 / 213 | 144 / 569 |
| 50 | 10 | 32,529 | 4,178 / 5,292 | 88 / 218 | 100 / 170 | 208 / 370 |
| 600 | 1 | 434,602 | 39,989 / 66,372 | 334 / 413 | 364 / 527 | 938 / 2,072 |
| 600 | 10 | 396,974 | 32,429 / 38,654 | 357 / 666 | 382 / 499 | 797 / 1,141 |
| 2,000 | 1 | 1,448,979 | 112,658 / 117,166 | 1,282 / 1,994 | 1,273 / 1,550 | 2,724 / 3,387 |
| 2,000 | 10 | 1,324,392 | 115,512 / 162,038 | 1,169 / 1,743 | 1,252 / 1,567 | 2,551 / 3,392 |
| 10,000 | 1 | 7,244,948 | 589,592 / 1,145,250 | 9,492 / 26,554 | 10,685 / 31,012 | 25,089 / 51,805 |
| 10,000 | 10 | 6,624,424 | 554,141 / 602,053 | 5,864 / 8,880 | 6,205 / 8,744 | 12,087 / 15,216 |

The 10,000-tool rows are synthetic offline stress only. The live model comparison used 600 and 2,000 tools. The top-10 retrieval results include a source-only server-description query; ranks and full measurements are in `results/tool-search-index-stress.json`. No provider-cache improvement is inferred from cache-read token counts.

The [MCP Scale Eval Checks workflow](../../.github/workflows/mcp-scale-evals.yml) runs deterministic tests, catalog enumeration, and this stress matrix without model credentials. Each successful run publishes its measurements in the `mcp-scale-stress` artifact.

## Recorded live evidence

The canonical serial qualification contains 54 Muse 1.3 Contributor trials: 36 read-only ticket and staged rollback trials at 600/2,000 tools on one server, plus 18 description-only disambiguation trials at 600/2,000 tools on ten servers. Each arm spans native, CLI, and hybrid with three trials per cell. All 54 passed both the task and state scorers; there were no subject failures, provider errors, or skips. A separate post-fix smoke passed one CLI read-only case and one CLI rollback case at 2,000 tools.

An earlier concurrency-3 attempt of the 600-tool, ten-server description case produced six passes and three OpenRouter “model not available” responses. These are provider availability failures, not subject failures. The report retains the earlier runner's raw `Subject` label; the qualification summary normalizes these three responses to infrastructure errors and documents that the classifier regression was added before serial retries. A first 600-tool/one-server command briefly used a substring filter that also matched `servers=10`; it was stopped immediately, flushed no JSONL report, and is excluded because its completed count is unknown.

Full event traces are retained as deterministic gzip JSONL under `results/`; `results/qualification-summary.json` contains every case's scorer, usage, metrics, report path, and compressed-report SHA-256. Corrected qualification totals are 493,572 uncached input, 31,915 output, and 1,167,856 cache-read tokens. Per-generation events report $0.058076 of effective cost; this is provider evidence, not invoice reconciliation. No causal cache benefit is inferred. The 54-trial run used the build before final CLI scope and deferred-server bound fixes; the separate two-case smoke used the rebuilt source at the end of that Framework qualification phase, whose 36 changed production Rust files and executable hash are recorded in the summary. The exact pre-fix binary was overwritten, so its source snapshot cannot be reconstructed from the recorded baseline commit alone. See the summary for baseline commit, dirty working-tree status, source hashes, token totals, separate retry lineage, and that phase's smoke distinction. Subsequent DB-backed platform checks and hosted fixes are recorded separately in [Hosted qualification](HOSTED_QUALIFICATION.md). To inspect a trace, run `gzip -dc results/live-read-rollback-600-1.jsonl.gz | less`.

## Shipping rebuild smoke (2026-10-21)

After the final behavior fixes, generated-contract refresh, and Bashkit security patch, the rebuilt study passed **6/6 Muse 1.3 Contributor trials**, with no failures, infrastructure errors, or skips. This is one read-only ticket and one state-verified staged rollback at 2,000 tools on one server for each surface. The [shipping summary](results/shipping-2026-10-21-summary.json) records the clean source commit, binary and lockfile hashes, exact commands, scorers, and separate [full traces](results/shipping-2026-10-21-live.jsonl.gz). A fresh [eight-row offline stress report](results/shipping-2026-10-21-stress.json) covers one and ten servers through 10,000 tools. The original 54-trial qualification and hosted evidence retain their separate source provenance.

Token totals across the two tasks per surface:

| Surface | Uncached input | Cache-read input | Output |
| --- | ---: | ---: | ---: |
| Native | 16,938 | 6,852 | 945 |
| CLI | 96,036 | 100,556 | 2,297 |
| Hybrid | 24,465 | 16,708 | 869 |
| Total | 137,439 | 124,116 | 4,111 |

These accumulate over all model calls. The CLI rollback requested the 2,000-tool server listing, with output cut at the shell's 64-KiB limit. Stable initial prompts do not bound model-requested output or total task tokens. Two tasks per surface do not establish a stable efficiency ranking or causal cache savings. The corrected provider-reported effective cost is $0.014814 across these six trials.

## Usage accounting correction (2026-10-23)

The original generic recursive event walk counted the same tokens in generation,
message, reason, turn, and session records: the published totals were **5× too
large**. It also missed Everruns' actual/estimated cost fields and reported zero.
The summaries above now use the shared [Rust usage projection](../common/README.md),
which selects each iteration once, retains folded compaction cost, and records
usage/capture coverage. Actual schema bytes come from captured driver requests,
not display summaries. The [correction manifest](results/usage-correction-2026-10-23.json)
records unchanged input archive hashes. Raw traces, source provenance, task
scores and fixture results remain unchanged; this is a recalculation, not a
model rerun. Hosted qualification already used per-generation accounting.

To reproduce one correction with the rebuilt study:

```bash
gzip -dc results/shipping-2026-10-21-live.jsonl.gz |
  ./target/debug/mcp_scale --reaccount-report - --output /tmp/corrected-usage.json
```

New runs also record accumulated/first/peak input, model-call and usage-record
counts, actual captured tool-array bytes, and provider cost coverage. Accumulated
input includes uncached, cache-read and cache-creation buckets; it is not a
single context-window size. Disjoint Everruns input must not be treated as
Mira's provider-dependent inclusive input convention.

## Sparse-session follow-up (2026-10-23)

The follow-up bounds CLI browsing to 64 entries / 12 KiB, recommends search
first, and suppresses incidental matches when an authorized exact operation
identity exists. Normal task queries retain the full ranked result window.
Normalized exact identities are indexed once, avoiding a catalog-wide
normalization/parsing pass on every search.

The [18-trial behavior matrix](results/sparse-2026-10-23-summary.json) passed
at 2,000 tools, with three repetitions per task/surface and no infrastructure
errors or skips. Mean input below includes uncached plus cache-read tokens:

| Surface | Mean input per task | Mean model calls | Largest request |
|---|---:|---:|---:|
| Bounded native | 11,955 | 4.50 | 4,102 |
| CLI | 19,404 | 5.67 | 5,346 |
| Hybrid | 20,177 | 4.50 | 5,651 |

CLI used 1.62× native's processed input in this matrix. None of its six trials
dumped server help. This is smaller than the earlier shipping smoke's 8.26×
ratio, whose CLI rollback requested the entire server listing; the small,
different run sizes do not establish a stable causal percentage saving.

After the behavior-preserving exact-lookup cache optimization, a clean-source
rebuild at `a6b4825af` passed a separate [6/6 final smoke](results/sparse-final-2026-10-23-summary.json).
Each surface used just **one distinct MCP operation** for the read and **four**
for rollback, from the 2,000-operation catalog. The complete runtime inventory
and the small used-operation set are separate measurements.

The [final eight-row offline stress report](results/sparse-2026-10-23-final-stress.json)
covers one and ten servers through 10,000 tools. In the recorded dev build
(`CARGO_PROFILE_DEV_DEBUG=0`, rustc 1.96.0, four logical CPUs), query p95 was
1.35–3.09 ms at 2,000 tools and 6.01–15.08 ms at 10,000 tools; index-build p95
was 99–101 ms and 453–500 ms respectively. The
[pre-cache report](results/sparse-2026-10-23-precache-stress.json) retained the
slower per-query identity normalization for comparison. These local timings
exclude model latency and MCP process/network startup. Ten-thousand-tool
coverage remains offline stress, not successful live model task qualification.

The [platform capability comparison](../platform-capability/README.md#recorded-muse-comparison-2026-10-23)
separately compares CLI with all 344 real platform operation schemas exposed
eagerly; it does not exercise Framework bounded discovery or hosted persistence.

## Final source follow-up (2026-10-24)

A separate [clean-source follow-up](results/followup-2026-10-24-summary.json)
at `18340609e` passed **6/6 Muse 1.3 Contributor trials**: one read-only ticket
and one state-verified staging rollback per surface, at 2,000 tools on one
server. No trials failed, encountered infrastructure errors, or skipped.
Each surface used one distinct MCP operation for reading and four for rollback.
The [full trace](results/followup-2026-10-24-live.jsonl.gz) retains every event;
the summary records source and binary hashes, complete usage/cost/request
capture coverage, and a successful reproduction of each raw usage row through
the shared Rust projection. Existing qualification archives remain separate.

Totals across the two tasks per surface:

| Surface | Uncached input | Cache-read input | Accumulated input | Model calls | Provider-reported cost |
| --- | ---: | ---: | ---: | ---: | ---: |
| Bounded native | 18,700 | 9,668 | 28,368 | 10 | $0.002095736 |
| CLI | 16,724 | 28,793 | 45,517 | 13 | $0.002017586 |
| Hybrid | 27,304 | 20,420 | 47,724 | 10 | $0.002978240 |

Native used less accumulated input here. CLI's provider-reported effective cost
was about 4% lower despite processing 1.60× native's input. Two tasks per
surface do not establish a stable cost ranking. Cache reads are observational:
the run overlapped the frozen platform comparison, so these counts establish
neither causal cache savings nor a universally cheapest surface.

The [final eight-row offline stress report](results/followup-2026-10-24-stress.json)
covers 50, 600, 2,000 and 10,000 tools on one and ten servers using the production
shared index. Query p95 across its three queries and both server partitions was
1.293–3.119 ms at 2,000 tools and 6.339–21.848 ms at 10,000 tools. This dev build
disabled debug information. The stress run overlapped a Framework rebuild and
platform model checks on the shared executor; CPU contention makes these
timings observational, and higher timings than earlier runs do not establish
an algorithmic regression. They exclude model latency and MCP process/network
startup. Ten-thousand-tool evidence remains offline stress, without a live
model accuracy claim.

## What gets measured

- **Tool evidence** requires at least one completed MCP operation in the Framework transcript.
- **State verification** reads the ticket result or validates the staged rollback result and preservation of the healthy production deployment. The read-only case fails if a mutating operation was called.
- Mira records model usage, elapsed time, iterations, and all raw Framework events for comparison across `native`, `cli`, and `hybrid`.
- The `description_only` case asks for “geothermal drilling telemetry,” a phrase found in the configured `telemetry` server description and absent from operation names. In ten-server trials, peer servers expose the same `get_ticket` operation name but cannot return the requested ticket, so the description must select the correct server.

The Rust fixture limits each page to 100 operations so catalog sizes above one page exercise real Framework pagination. Ten-server cases split the declared catalog size across servers. Synthetic distractor operations exist only to create catalog scale; they do not model thousands of independently designed services.

## Historical prototype evidence

`historical/python-prototype/` preserves the previous Python gateway, retrieval comparison, scripts, runtime probe, and result files. Those reports describe the old prototype and are not measurements from this Rust Mira study. The Python evaluator and gateway are archived and are no longer the canonical runner or fixture.
