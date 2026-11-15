---
title: Evaluation and traces
description: Project portable session traces and run shared rule, judge and decision scorers in offline cases or bounded online listeners.
---

Portable trace projection uses the same pure event fold as the Server's trace index:

```rust
# async fn inspect(session: &everruns::Session) -> Result<(), Box<dyn std::error::Error>> {
use everruns::trace::Trace;

let trace = Trace::project(&session.events_after(0).await?)?;
println!("{}", serde_json::to_string_pretty(&trace)?);
# Ok(())
# }
```

Supply a complete, sequence-ordered prefix from one session. Mixed sessions, missing stored sequences and duplicate/out-of-order events are rejected. The result contains turns and model/tool/approval/delegation steps with bounded previews. Full model prompts and tool bodies are not retained in the projection. Treat trace previews as application data when exporting them. The indexed Server API and trace viewer are host features.

## Offline evaluation

Enable `evaluation`. `Evaluator` validates and runs the same deterministic scoring rules the Server uses. Supported rules include contains/not-contains, regex, tool use and argument matching, call/turn/token bounds, JSON schema and citation faithfulness. File checks use explicitly collected session workspace evidence.

```rust
use everruns::evaluation::{Evaluator, Scorer};

let evaluator = Evaluator::new(vec![
    Scorer::Contains { text: "42".into(), weight: 1.0 },
    Scorer::TurnsWithin { max: 1, weight: 1.0 },
])?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`run(engine, agent, cases, concurrency)` creates a fresh real session per case, runs each input in order, captures evidence and returns results in case order. Concurrency is explicitly bounded to 1–32. Each case has a per-input deadline, and timed-out turns are cooperatively cancelled. Results retain session IDs, exact build registrations, evidence, ordered scores and failures for debugging. For a durable Engine, use `run_registered(engine, build, cases, concurrency)` with an application-retained `AgentBuild`; re-register it after restart.

Use `.judge(Model)` for rubric/citation judges and `.decisions(Decisions)` for typed yes/no grading. Judge prompt construction and response parsing are shared with Server. Providers and credentials are explicitly selected by the application. Hosted model IDs require a custom `Grader`; they are never silently translated into another account's model. Missing evidence/models and provider failures are errors. `Grader` also supports application-specific external evidence.

The [complete evaluation example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/service_evaluation.rs) runs a real arithmetic agent, prints scores and projects its trace:

```bash
cargo run -p everruns --features openai,evaluation --example service_evaluation
```

## Online scoring

Install `OnlineScoring::new(evaluator, sink)` with `Engine::builder().listener(...)`. Implement `ScoreSink` to retain/export grades. It uses the same evaluator as offline runs and executes on the Engine's bounded observer queue, outside turn execution.

The listener bounds retained turns and evidence size; overflow, unsuccessful turns and grading failures are reported to the sink. Monitor Engine observer drop statistics. Online scoring is observational and must not be used as a durable audit ledger; replay canonical events for complete historical grading. `Engine::shutdown(deadline)` flushes listeners within a caller-selected deadline. Server evaluation definitions, history/reporting storage, durable scoring jobs and dashboards remain host concerns.
