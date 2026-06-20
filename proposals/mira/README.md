# mira — eval framework prototype

A Rust-first, code-first evaluation framework for agents and tools. Design lives
in [`SPEC.md`](./SPEC.md); this is a runnable prototype that proves the model
against the real `everruns-runtime`.

> Standalone workspace, intentionally excluded from the everruns build. For
> handover, swap the everruns path deps in `Cargo.toml` for published crates
> (`everruns-runtime = "0.15"`, …).

## Two processes, one protocol

You **define evals in code** in a small program (the *server*) that calls
`mira::serve(...)`. A generic CLI (`mira`, the *host*) compiles and spawns it,
then drives everything generic — selection, the model matrix, aggregation,
saving, checkpoints, visualization — over newline-delimited JSON on stdio,
MCP-style. Provider API keys stay in the server's env and never cross the wire.

```
┌────────────┐   list / run  (JSON / stdio)   ┌──────────────────────────┐
│  mira CLI  │ ─────────────────────────────▶ │  your eval program       │
│  (host)    │ ◀───────────────────────────── │  defines evals + serve() │
│ select,    │   results / progress           │  owns runtime + scoring  │
│ matrix,    │                                └──────────────────────────┘
│ aggregate, │
│ checkpoint │
└────────────┘
```

## Run it (offline, no API key)

```bash
cargo build --bins        # builds `mira` (host) and `demo_evals` (server)

mira=./target/debug/mira
server="--cmd ./target/debug/demo_evals"      # or: --bin demo_evals (compiles via cargo)

$mira $server list                            # advertised evals/samples/scorers/models
$mira $server run                             # all cells (sim runs; real cells skip)
$mira $server run greet                       # substring filter on eval/sample@model
$mira $server run --tag smoke                 # select by tag
$mira $server run --models sim --out r.json   # restrict matrix + write JSON
$mira $server run --checkpoint ck.json        # resumable: re-run skips done cells
```

Anthropic/OpenAI matrix cells advertise as *unavailable* and skip unless
`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` are set, so the default run is green.

Example:

```
server mira · protocol 0.1 · 2 evals
── cases ──
  [PASS] greet/hi@sim  (100%)
         ✓ succeeded — no error
         ✓ contains — found "42"
  [SKIP] greet/hi@anthropic/claude-haiku-4-5
── matrix (passed/ran) ──
  eval     sim   anthropic/claude-haiku-4-5   openai/gpt-5.5
  greet    1/1                            —                —
2 passed / 2 ran (0 failed, 4 skipped)
```

## Layout

| Piece | Where |
|-------|-------|
| protocol (messages, framing) | `src/protocol.rs` |
| server — `serve(evals)` | `src/server.rs` |
| host — spawn + drive | `src/host.rs` |
| `Subject` (`RuntimeSubject`) | `src/subject.rs` |
| `Scorer` (built-ins + `model_graded`) | `src/scorer.rs` |
| `Eval` builder | `src/eval.rs` |
| in-process runner + `run_cell` | `src/runner.rs` |
| reporting (matrix + JSON) | `src/report.rs` |
| **host CLI** | `src/bin/mira.rs` |
| **demo server** | `src/bin/demo_evals.rs` |

See `SPEC.md` §9 for what's deferred (the `#[eval]` macro sugar, `report.html`
viewer, `ToolSubject`/`CliSubject`, JUnit/TAP).
