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

## Recorded live evidence

The canonical serial qualification contains 54 Muse 1.3 Contributor trials: 36 read-only ticket and staged rollback trials at 600/2,000 tools on one server, plus 18 description-only disambiguation trials at 600/2,000 tools on ten servers. Each arm spans native, CLI, and hybrid with three trials per cell. All 54 passed both the task and state scorers; there were no subject failures, provider errors, or skips. A separate post-fix smoke passed one CLI read-only case and one CLI rollback case at 2,000 tools.

An earlier concurrency-3 attempt of the 600-tool, ten-server description case produced six passes and three OpenRouter “model not available” responses. These are provider availability failures, not subject failures. The report retains the earlier runner's raw `Subject` label; the qualification summary normalizes these three responses to infrastructure errors and documents that the classifier regression was added before serial retries. A first 600-tool/one-server command briefly used a substring filter that also matched `servers=10`; it was stopped immediately, flushed no JSONL report, and is excluded because its completed count is unknown.

Full event traces are retained as deterministic gzip JSONL under `results/`; `results/qualification-summary.json` contains every case's scorer, usage, metrics, report path, and compressed-report SHA-256. Qualification rows sum to 2,467,860 input tokens, 159,575 output tokens, and 5,839,280 cache-read tokens. The provider returned `cost_usd: 0.0`; actual billing is unknown, so this is not a claim that the run was free. No cache benefit is inferred. The 54-trial run used the build before final CLI scope and deferred-server bound fixes; the separate two-case smoke used the rebuilt source at the end of that Framework qualification phase, whose 36 changed production Rust files and executable hash are recorded in the summary. The exact pre-fix binary was overwritten, so its source snapshot cannot be reconstructed from the recorded baseline commit alone. See the summary for baseline commit, dirty working-tree status, source hashes, token totals, separate retry lineage, and that phase's smoke distinction. Subsequent DB-backed platform checks and hosted fixes are recorded separately in [Hosted qualification](HOSTED_QUALIFICATION.md). To inspect a trace, run `gzip -dc results/live-read-rollback-600-1.jsonl.gz | less`.

## What gets measured

- **Tool evidence** requires at least one completed MCP operation in the Framework transcript.
- **State verification** reads the ticket result or validates the staged rollback result and preservation of the healthy production deployment. The read-only case fails if a mutating operation was called.
- Mira records model usage, elapsed time, iterations, and all raw Framework events for comparison across `native`, `cli`, and `hybrid`.
- The `description_only` case asks for “geothermal drilling telemetry,” a phrase found in the configured `telemetry` server description and absent from operation names. In ten-server trials, peer servers expose the same `get_ticket` operation name but cannot return the requested ticket, so the description must select the correct server.

The Rust fixture limits each page to 100 operations so catalog sizes above one page exercise real Framework pagination. Ten-server cases split the declared catalog size across servers. Synthetic distractor operations exist only to create catalog scale; they do not model thousands of independently designed services.

## Historical prototype evidence

`historical/python-prototype/` preserves the previous Python gateway, retrieval comparison, scripts, runtime probe, and result files. Those reports describe the old prototype and are not measurements from this Rust Mira study. The Python evaluator and gateway are archived and are no longer the canonical runner or fixture.
