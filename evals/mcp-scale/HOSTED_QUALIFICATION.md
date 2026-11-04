# Hosted MCP qualification, 2026-10-19

This supplements the [Rust/Mira study](README.md) with manual execution against the DB-backed platform. It uses real PostgreSQL, Valkey, NATS, API, worker and UI, plus the [Rust HTTP fixture](HTTP_FIXTURE.md). The model is OpenRouter `meta/muse-spark-1.3-contributor`. All records, servers, plugin contents and authorization denials are synthetic. No Yolop changes are required.

The six [manual cases](../../knowledge/test-cases/agents/large_mcp_catalogs/index.md) cover pagination, source metadata and colliding names, native/CLI/hybrid parity, dynamic MCP and plugin lifecycle, discovery bounds, and deferred CLI loading. Agent configuration and most turns use the normal HTTP API; Playground submission, rendered conversations, traces and cost views are also inspected in Chromium. This is targeted hosted integration evidence, not a statistical accuracy benchmark.

## Findings and fixes

The first native run exposed a hosted-only replay defect: stored tool results are reconstructed as JSON text, while the native disclosure reader accepted only structured objects. It now accepts both forms, decoding one JSON-text layer and leaving ordinary text opaque. A regression covers bounded and legacy discovery.

The hosted catalog initially omitted configured server descriptions: deferred placeholders used the hostname, and listed definitions lacked the source metadata used by the index. Final-source hosted retests now pass through both paths: native source refinement selects the matching server, and CLI search returns description matches. A description refresh makes the old phrase stop matching and the new phrase select the server. Preset descriptions remain a fallback. The worker transport carries canonical source names and descriptions alongside tool definitions; unrelated metadata extensions stay server-side. Search still resolves selected identities against the current registry; metadata cannot grant execution authority.

Deferred CLI listing also preserves that source context on its loaded children. Repeating a description-only query can identify the loaded server without listing it again. Risk hints and unrelated child metadata are preserved.

Intermittent provider `model_unavailable` failures are retained separately. A turn that executes the expected calls and then fails at the provider is not counted as a completed trial; the affected checks are repeated serially on the same model.

## Results

All six hosted case groups passed the tested submatrices. TC001 verified 600- and 2,000-tool discovery, a post-restart API native read, and a native 2,000-tool read submitted through Playground. TC002 verified colliding schemas, scope, source-description-only selection through native and CLI, and metadata refresh. TC003 verified CLI and hybrid parity plus post-revocation denials. TC004 verified plugin/MCP lifecycle, compaction, and fresh-worker replay. TC005 verified discovery bounds/faults and recovery of the previous complete 600-tool snapshot after a later-page HTTP 503. TC006 verified description-based deferred loading in batches of eight then two, scope, stable native tools/instructions, and atomic rejection of a 9,800 eager plus 500 deferred aggregate overflow with the deferred source left pending.

Initial parity and boundary captures used the JSON-replay-fix build. Source-description selection and refresh, restart recovery and replay, deferred loading and overflow, and the final Playground read were repeated after rebuilding both final binaries. The manifest records both build phases. The final run used the canonical startup script with a local launcher that executes the verified prebuilt binaries, avoiding another Cargo feature-unification rebuild.

These are targeted synthetic observations, not exhaustive provider, topology, or surface cross-products. Hosted 10,000-tool coverage is protocol enumeration, indexing, and limit testing; successful model tasks were tested through 2,000 tools. The final captures are in the versioned [hosted evidence bundle](results/hosted-manual-2026-10-19.jsonl.gz) and [manifest](results/hosted-manual-2026-10-19.manifest.json); local raw captures and screenshots are not part of the public repository.

## Prompt and token observations

The earlier 2,000-operation native cold turn made three model calls and used 10,638 uncached input, 2,161 cache-read and 422 output tokens. Its warm fresh read made two calls and used 605 uncached input, 9,442 cache-read and 209 output tokens. A final-source post-restart API read used 10,948 uncached input, zero cache-read and 270 output tokens; the separate Playground-submitted read used 12,011 uncached input, zero cache-read and 150 output tokens. These are provider-reported disjoint input buckets from different calls. They do not establish a causal savings claim or guarantee prompt-cache hits.

The actual 2,000-tool wire request started with seven baseline functions, then exposed one optional MCP function. The 600-tool request exposed two optional MCP functions. Both stay within the configured optional 32-schema/64-KiB budget. Complete catalogs are discovered by the runtime; their full schemas are not sent up front.

Unchanged native turns preserve the serialized tools, system instructions and every previous input item. The metadata-refresh comparison preserved tools, instructions and all 17 prior input items. A pure CLI experiment preserves identical tool and instruction hashes as ten deferred servers load eight then two; description-only search also loaded ten attached servers in batches of eight and two. CLI discovery changes its execution catalog without inserting the inventory into the system prompt.

Dynamic plugin attachment appends its instructions: the existing 6,471-character instruction string is an exact prefix of the new 6,836-character string. Removal restores the baseline instructions. Native additions and schema redefinitions change the wire tool list on this endpoint. Removed definitions can remain on the wire under the existing warm-hold behavior, while direct CLI execution is checked against the current catalog and fails closed. Prompt stability, current authority and provider cache hits are measured separately.

Compaction uses the real cost-pressure masking strategy on this model; the replayed lifecycle session recorded 23 compactions. This does not qualify a native provider compaction checkpoint or imply that masking always reduces token count; the deliberately small synthetic outputs can grow when replaced by a mask marker.

## Scope of the evidence

Hosted boundary probes cover HTTP per-server count, full-response bytes, page count, cursor/name faults and late-page failures. The preview endpoint returns HTTP 200 with an empty tool list on discovery failure; the detailed reason is in runtime diagnostics. A warm DB-backed turn also proves recovery of the prior complete catalog after a later-page 503.

Final-source focused tests passed 168 server MCP tests, 64 server search tests (overlapping the MCP suite), two worker metadata tests, and ten deferred CLI tests. Framework-level aggregate catalog/cache limits and stdio protocol behavior have separate deterministic automated coverage; those tests are not hosted DB/API/UI observations. The inline `actsAs:none` hosted fixture does not qualify OAuth refresh, the platform's persistent identity cache, every provider, or live third-party MCP servers. Synthetic model task qualification remains bounded to 2,000 operations; 10,000-operation evidence is protocol, search and limit testing.

The hosted worker transport retains its existing 16-MiB message limit. The near-limit CLI check uses this fixture's 9,800-operation catalog; it does not establish that arbitrary schemas at the tool-count ceiling fit through that transport.
