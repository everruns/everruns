---
title: Coding Review Agent
description: Tool-based code inspection, a tightly scoped execution tool, and distinguishing test failure from tool failure.
---

[Browse the complete example](https://github.com/everruns/everruns/tree/main/examples/coding-review-agent).

Review a refund implementation against a written contract, then execute a bundled regression test before reporting a defect. A proposed test is not treated as proof: the agent gets actual compiler and test output.

## What you learn

Tool-based code inspection, a tightly scoped execution tool, and distinguishing test failure from tool failure.

## Scenario and expected outcome

Two refunds of 1,000 cents are requested against a 1,000-cent payment. Each individual call caps its own amount, but the implementation retains no cumulative refunded balance.

The regression fails with `left: 2000` and `right: 1000`. The review should connect that observed failure to the missing cumulative-refund state, describe the over-refund risk, and propose a minimal fix. It must not claim the code was changed or fixed.

## Run it

Install Rust/Cargo (including rustc), clone the repository, and run from its root. These folders are self-contained **within the workspace**: their Cargo manifests reference the local Framework crates, so copying one folder alone is not sufficient.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export ANTHROPIC_API_KEY="your-key"
cargo run -p everruns-coding-review-agent
```

The configured model is `claude-sonnet-5`. Provider access and funded credits are required; a model identifier alone does not grant access. Keep keys in your environment, not in source control. Missing variables, provider errors, or unsuccessful turns exit nonzero.

Try a contrasting question:

```bash
cargo run -p everruns-coding-review-agent -- "Read the contract and test. Reproduce the cumulative refund bug and explain the smallest safe fix."
```

## Build the agent

This is the actual builder from `src/main.rs`. The prompt is `src/instructions.md`. Tools/capabilities supply evidence and actions; the model chooses how to use them.

```rust
let agent = Agent::builder()
    .name("coding-review-agent")
    .instructions(include_str!("instructions.md"))
    .provider(everruns_anthropic::provider("anthropic", api_key))
    .model(MODEL)
    .max_iterations(12)
    .tool(tools::inspect_change())
    .tool(tools::run_regression())
    .build()?;
```

## Send, observe, and wait

The Framework interaction stays readable in `main.rs`. The shared demo helper subscribes before sending, filters events to this turn, shows bounded tool previews, waits for completion, and rejects unsuccessful turns. It changes presentation only; use `session.send_and_wait(question).await?` when you do not need the live tool timeline.

```rust
let engine = Engine::new();
let session = engine.create(agent);
println!("MODEL: {MODEL}");
demo::run(&session, question).await?;
```

This engine is in-memory. It does not demonstrate durable session storage; the Everruns Support example can explain that API, but does not itself persist its session.

## How the tools work

`inspect_change` can read only `sample_payment.rs`, `contract.md`, and `regression.rs`. `run_regression` invokes `rustc --test` on that fixed trusted fixture, runs the temporary binary, and returns its exit code and assertion output. Compilation and execution have timeouts; temporary files are removed automatically.

## Validate the behavior

```bash
cargo test -p everruns-coding-review-agent
python3 examples/coding-review-agent/src/render_demo.py --check
```

The offline test compiles and runs the bundled regression and asserts the observed failure. `cargo test -p everruns-coding-review-agent` passes because it verifies reproduction of the intentionally buggy fixture. A failing subprocess is expected evidence, not a failing example test.

CI runs these offline checks without provider credentials. Live model behavior is evaluated separately; passing tests is not proof of answer quality.

## Demo and recording

![Coding Review Agent recorded run](https://raw.githubusercontent.com/everruns/everruns/main/examples/coding-review-agent/src/demo.gif)

Read the [captured transcript](https://github.com/everruns/everruns/blob/main/examples/coding-review-agent/src/demo.txt) at your own pace. The GIF is a paginated
replay of an actual provider run, with waiting time removed. It is not
interactive and does not show model reasoning. Result excerpts are shortened
only for display.

With credentials exported and Python 3, VHS, ffmpeg, and a VHS-compatible browser installed:

```bash
cd examples/coding-review-agent
bash src/record.sh
```

The script captures a successful run, generates correctly wrapped pages and page durations, and renders `src/demo.gif`. It preserves the previous transcript when the provider run fails. To replay an existing transcript without another model call, run `(cd src && python3 render_demo.py && vhs demo.tape)`. `src/demo.txt` retains the displayed output; `.demo-pages/` is generated and ignored. Inspect results before sharing: public/demo data is safe here, but adapting tools may expose private data.

## Adapt it

Replace the fixture with a trusted review checkout and a restricted test selection. Use a sandbox before accepting arbitrary repositories, generated code, or model-selected commands. Add a second verified run after applying a fix in a separate approved workflow.

## Boundaries

This is one deliberately buggy, trusted fixture—not a general-purpose coding agent. The execution tool takes no shell command or user-supplied path, and cannot edit code. Rust including `rustc` must be installed.

## Source map

`src/main.rs`: agent and session; `src/tools.rs`: file allowlist and fixed regression execution; `src/sample_payment.rs`: buggy implementation; `src/contract.md`: required behavior; `src/regression.rs`: executable reproduction. `examples/demo-support` handles shared terminal presentation; `src/record.sh` and `src/render_demo.py` handle recording.
