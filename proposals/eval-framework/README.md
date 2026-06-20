# eval framework — prototype

A Rust-first, code-first evaluation framework for agents and tools. Design lives
in [`SPEC.md`](./SPEC.md); this is a runnable prototype that proves the core
model against the real `everruns-runtime`.

> Standalone workspace, intentionally excluded from the everruns build. Crate
> name `evals` is a placeholder. For handover, swap the three path deps in
> `Cargo.toml` for published crates (`everruns-runtime = "0.15"`, etc.).

## Run it (offline, no API key)

```bash
cargo run --example coding_eval                  # all evals, sim cell
cargo run --example coding_eval -- greet         # selective: substring filter
cargo run --example coding_eval -- --tag smoke   # selective: by tag
cargo run --example coding_eval -- --json out.json
```

The Anthropic/OpenAI matrix cells skip automatically unless `ANTHROPIC_API_KEY`
/ `OPENAI_API_KEY` are set, so the default run is green out of the box.

Example output:

```
── cases ──
  [PASS] greet/hi@sim  (100%)
         ✓ succeeded — no error
         ✓ contains — found "42"
         ✓ turns_within — 1 <= 3

── matrix (passed/total) ──
  eval       sim
  greet      1/1
  judge      1/1
  [SKIP] greet/hi@anthropic/claude-haiku-4-5 (no API key)

2 passed / 2 total (0 failed, 4 skipped)
```

## Shape

```
Eval = Dataset(Sample…) + Subject + [Scorer…]  ×  model matrix
```

| Piece | Prototype | Where |
|-------|-----------|-------|
| `Subject` | `RuntimeSubject` (real runtime) | `src/subject.rs` |
| `Scorer` | `contains`/`regex`/`tool_called`/`model_graded`/closures | `src/scorer.rs` |
| `Eval` builder | inline cases or `Dataset::jsonl` | `src/eval.rs` |
| matrix + selection | `Runner` | `src/runner.rs` |
| reporting | terminal matrix + JSON | `src/report.rs` |
| end-to-end | offline + real-provider matrix | `examples/coding_eval.rs` |

See `SPEC.md` §8 for what's deferred to implementation (the `#[eval]` macro
+ `libtest-mimic` harness, `ToolSubject`/`CliSubject`, HTML viewer).
