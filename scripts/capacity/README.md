# Chat capacity benchmark

This is a platform benchmark with synthetic users and `llmsim`. It runs real
authentication, Console command admission, Agent API execution, Work Chat's BFF,
SSE, the worker, PostgreSQL, NATS, Valkey and a Caddy HTTPS/HTTP2 proxy. It never
uses a real LLM. Interactive sign-in, browser rendering and WAN conditions are
outside this benchmark.

Run from the repository root after installing the normal local development
prerequisites (including Python >=3.11, native PostgreSQL, Caddy >=2.11.4,
Node >=24, pnpm and Rust):

```bash
python3 -m venv .venv-capacity
.venv-capacity/bin/pip install -r scripts/capacity/requirements.txt
CAPACITY_PYTHON="$PWD/.venv-capacity/bin/python" just bench-chat-capacity
CAPACITY_PYTHON="$PWD/.venv-capacity/bin/python" just bench-chat-capacity --qualification
```

The default is a five-user burst and a 40-second stream-lifetime smoke on each
path. Qualification runs, separately for
Console, Agent API and Work Chat:

1. 100, 250 and 500 connected users, each for 60 seconds.
2. 1,000 connected users for 30 minutes, one turn per user every 60 seconds.
3. A simultaneous 1,000-turn burst, reported separately from steady load.
4. A recovery case with repeated acknowledgments, a stream reconnect wave and an
   abrupt server restart. Transient errors remain in the report; fault-case
   admission has its own 15-second budget.

Each actor has a distinct server-verified identity and an independent SSE cursor.
Each has its own HTTP client/connection pool, reusing its HTTP/2 connection for
SSE, sends and polls. Users do not share transport connections or cookie jars.
Console runs the managed Platform Chat Agent on Bashkit Worker, with an explicit
verified `llmsim` session model. Agent API and Work Chat use a dedicated Agent on
the Base harness with the session filesystem capability and direct communication.
Five tenant organizations share one server and worker. Work Chat includes the
host's background reads; Console includes periodic and coalesced lifecycle-driven
thread-list reads. Deterministically seeded short answers, long answers,
thinking and local-file tool rounds reproduce realistic delays and event volume.
Console's file-research variant uses directly available `read_file` calls;
the other paths use directory/read/stat operations. Both perform three rounds
and four local tool calls. Runtime facts remain context for scenario selection.
The long-answer shape itself takes over 30 seconds, so the default steady interval
is 60 seconds (about 16.7 arrivals/second at 1,000 users). A shorter closed-session
interval would miss its schedule even without platform contention.
Setup and durable-event verification run outside the timed phase. Completion is
`turn.completed`, correlated through its input message and turn ID. Work Chat's
public projection hides the input-to-turn reference, so it uses the matching
input echo and one outstanding turn; a failed turn stops that actor's schedule.

The coordinator builds optimized fixture/worker binaries and the production Work
Chat host, creates random disposable databases under an unused port prefix, and
stops its processes on exit. First builds can be slow. Builds finish before timed
load starts. `--skip-build --fixture-bin <binary>` reuses an already built fixture;
`--help` lists topology, duration and workload overrides. Retained prefix-scoped
infrastructure data follows the repository's normal explicit reset contract.

On a Mac mini, run the same command in a checkout with those prerequisites. Raise
the file descriptor hard limit to at least 4096 if necessary. Linux records
proportional resident memory; macOS falls back to RSS, which conservatively
double-counts shared pages. A local run measures that machine and includes
generator contention. For deployment qualification, run the generator separately
against an isolated installation using a private actor manifest and
`python scripts/load-test.py --capacity --help`. Coordinated shards require the
same run ID and start time. `python -m capacity.merge` (from `scripts/`) merges raw
samples and rejects inconsistent or missing shards; it never averages percentiles.

## Results and interpretation

Results default to ignored `target/chat-capacity/<timestamp>/results/`. Each case
contains raw turn samples, resource samples, shard reports and a merged report.
The aggregate summary retains failures. The sibling `private/` directory contains
credentials and must never be uploaded or committed; process logs also stay local.
Publish only inspected result files. Metadata records the source revision, host
limits, topology, simulator and worker configuration. Comparisons require matching
settings and a quiet host.
The fixture disables the global per-IP API limiter and raises setup/resource
creation quotas because all synthetic clients share one loopback address. These
settings are recorded; a pass establishes capacity for that topology, not an
out-of-the-box deployment or its production rate-limit policy.

Steady admission must reach 99.9%, acknowledgment p95 must stay below 500 ms, every
admitted turn must complete, and persisted inputs must appear exactly once and be
observed on SSE. A separate durable audit verifies final answer size, tool/LLM
rounds and simulator pacing, rejecting the simulator's tiny fallback response.
Setup, tool, schedule, background-read and unexpected stream
failures fail the gate. Resource coverage and process liveness are checked, with a
75% effective-memory budget. For sufficiently sampled steady runs, late pickup
p95 cannot exceed twice early pickup p95 or one second, whichever is larger.
Incomplete turns or missed arrival schedules independently fail. The queue trend
is explicitly unmeasured for small samples; a smoke is never a capacity result.
Generator arrival lag p95 must stay below 100 ms; a generator that cannot sustain
the requested schedule cannot establish platform capacity.
The simultaneous burst has a separately reported, configurable five-second
acknowledgment budget; recovery uses fifteen seconds. Neither relaxes the steady
load's 500 ms gate.
Requested reconnects must restore every stream, and a requested server restart
must actually occur inside the measured phase; an early-ending case cannot
silently skip its faults and pass.

The benchmark refuses known production hosts and requires an explicit isolated
target and enabled `llmsim` models for every tenant before sending messages.
The local coordinator strips provider credentials, uses fresh databases and
full authentication. A synthetic RS256 issuer primes only the trusted embedding
host's public key cache; JWT signature, expiry, audience and authorization checks
still execute. The production loopback-JWKS SSRF restriction remains enabled.

A failed 1,000-user run is a valid benchmark result. Investigate acknowledgment,
pickup, simulator duration, poll traffic, database pressure and generator CPU
before attributing failure to SSE. This benchmark does not compare SSE with
WebSockets; changing transport requires a controlled comparison on equivalent
durable delivery semantics.

Offline correctness checks: `python scripts/test-load-test.py`.
