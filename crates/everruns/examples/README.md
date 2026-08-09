# `everruns` examples

These examples use the application-facing [`everruns`](../README.md) crate and
`gpt-5.6-terra`. Set `OPENAI_API_KEY` before running them.

| Example | What it demonstrates | Run |
|---|---|---|
| [`hello.rs`](hello.rs) | Minimal agent, typed tool, turn result, and event observation | `cargo run -p everruns --features openai --example hello` |
| [`production_agent.rs`](production_agent.rs) | Tool safety boundary, multi-turn use, JSONL persistence, and resume | `cargo run -p everruns --features openai,jsonl --example production_agent` |
| [`github_monitor.rs`](github_monitor.rs) | Host-owned background work that wakes an agent when it finishes | `cargo run -p everruns --features openai --example github_monitor -- --simulate` |
| [`subagents.rs`](subagents.rs) | Concurrent child agents managed by an application-owned task registry | `cargo run -p everruns --features openai --example subagents` |
| [`observe_and_cancel.rs`](observe_and_cancel.rs) | Live event streaming and cooperative cancellation | `cargo run -p everruns --features openai --example observe_and_cancel` |

The GitHub monitor's default `--simulate` mode skips GitHub access but still
uses the configured model. Its explicit live mode requires an authenticated
GitHub CLI:

```text
cargo run -p everruns --features openai --example github_monitor -- --live OWNER/REPO PR_NUMBER
```
