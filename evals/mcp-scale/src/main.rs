mod accounting;
#[path = "../../common/everruns_usage.rs"]
mod event_usage;
mod fixture;
mod scorers;
mod stress;
mod subject;

use std::sync::Arc;

use mira::scorer::succeeded;
use mira::{CaseOutcome, Dataset, ErrorKind, Eval, Target, Trial, eval};
use serde_json::json;

const DATASET: &str = include_str!("../dataset.jsonl");
const SIZES: [&str; 4] = ["50", "600", "2000", "10000"];
const SERVER_COUNTS: [&str; 2] = ["1", "10"];
const SURFACES: [&str; 3] = ["native", "cli", "hybrid"];

fn targets() -> Vec<Target> {
    let spec = std::env::var("EVERRUNS_EVAL_TARGETS").unwrap_or_default();
    if spec.trim().is_empty() {
        return vec![Target::cloud(
            "openrouter",
            "meta/muse-spark-1.3-contributor",
            "OPENROUTER_API_KEY",
        )];
    }
    spec.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.split_once('/') {
            Some(("anthropic", model)) => Target::anthropic(model),
            Some(("openai", model)) => Target::openai(model),
            Some(("openrouter", model)) => Target::cloud("openrouter", model, "OPENROUTER_API_KEY"),
            Some((provider, model)) => Target::new(entry, provider, model),
            None => Target::new(entry, entry, ""),
        })
        .collect()
}

fn positive_env(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(fallback)
}

#[eval]
fn mcp_scale() -> Eval {
    let dataset = Dataset::from_jsonl_str(DATASET).expect("embedded MCP dataset must parse");
    Eval::new("mcp_scale")
        .describe("MCP catalog discovery, server disambiguation, and state-verified operations across native, CLI, and hybrid surfaces")
        .dataset(dataset)
        .targets(targets())
        .trials(positive_env("EVERRUNS_EVAL_TRIALS", 3))
        .axis("catalog_size", SIZES)
        .axis("servers", SERVER_COUNTS)
        .axis("surface", SURFACES)
        .subject(subject::McpScaleSubject)
        .scorer(succeeded())
        .scorer(scorers::tool_evidence())
        .scorer(scorers::state_verified())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--mcp-fixture") {
        return fixture::serve(&args);
    }
    if let Some(input) = value_after(&args, "--reaccount-report") {
        let output = value_after(&args, "--output")
            .ok_or_else(|| std::io::Error::other("--reaccount-report requires --output"))?;
        return accounting::reaccount_report(&input, &output);
    }
    if args.iter().any(|arg| arg == "--offline-catalog") {
        for size in [50, 600, 2_000, 10_000] {
            let tools = fixture::catalog(size);
            let unique = tools
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect::<std::collections::HashSet<_>>()
                .len();
            println!(
                "{} tools, {} unique names, first={}, last={}",
                tools.len(),
                unique,
                tools.first().unwrap()["name"],
                tools.last().unwrap()["name"]
            );
            let ten_server_total = (0..10)
                .map(|index| subject::server_catalog_size(size, 10, index))
                .sum::<usize>();
            println!("  ten-server partition: {ten_server_total} tools");
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--offline-stress") {
        let output = value_after(&args, "--output").unwrap_or_else(|| "results/stress.json".into());
        stress::run(&output)?;
        println!("offline ToolSearchIndex stress report: {output}");
        return Ok(());
    }
    if !args.iter().any(|arg| arg == "--run") {
        return mira::Study::registered().serve().await;
    }
    let value_of = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };
    let filter = value_of("--filter");
    let sample_ids =
        value_of("--sample").map(|value| value.split(',').map(str::to_owned).collect());
    let (outcomes, skipped) = run_concurrently(
        filter,
        sample_ids,
        positive_env("EVERRUNS_EVAL_CONCURRENCY", 3),
    )
    .await;
    for outcome in &outcomes {
        println!(
            "{}  {}",
            if outcome.passed { "PASS" } else { "FAIL" },
            outcome.key()
        );
        for score in &outcome.scores {
            if outcome.passed && (score.na || score.pass) {
                continue;
            }
            println!(
                "        {:<24} {}  {}",
                score.scorer,
                if score.na {
                    "n/a".into()
                } else {
                    format!("{:.2}", score.value)
                },
                score.reason
            );
        }
        if let Some(error) = &outcome.transcript.error {
            println!("        error: {error}");
        }
    }
    let infra = outcomes
        .iter()
        .filter(|outcome| {
            outcome.transcript.error_kind == ErrorKind::Infra && outcome.transcript.error.is_some()
        })
        .count();
    let passed = outcomes
        .iter()
        .filter(|outcome| {
            outcome.passed
                && !(outcome.transcript.error_kind == ErrorKind::Infra
                    && outcome.transcript.error.is_some())
        })
        .count();
    println!(
        "\n{passed} passed, {} failed, {infra} infra, {skipped} skipped, {} executed",
        outcomes.len().saturating_sub(passed + infra),
        outcomes.len()
    );
    println!("tool evidence: see each scored Framework transcript");
    if let Some(path) = value_of("--runs-report")
        .or_else(|| std::env::var("EVERRUNS_EVAL_RUNS_REPORT").ok())
        .filter(|path| !path.trim().is_empty())
    {
        let lines = outcomes.iter().map(|outcome| json!({
            "case":outcome.key(), "passed":outcome.passed,
            "scores":outcome.scores.iter().map(|score| json!({"name":score.scorer,"passed":score.pass,"na":score.na,"value":score.value,"reason":score.reason})).collect::<Vec<_>>(),
            "error":outcome.transcript.error, "error_kind":format!("{:?}", outcome.transcript.error_kind),
            "duration_ms":outcome.transcript.timing.duration_ms,
            "usage":outcome.transcript.usage, "tool_calls":outcome.transcript.tool_calls,
            "metrics":outcome.transcript.metrics, "metadata":outcome.transcript.metadata,
            "events":outcome.transcript.events,
        }).to_string()).collect::<Vec<_>>().join("\n");
        std::fs::write(&path, lines + "\n")?;
        println!("runs report: {path}");
    }
    if outcomes.is_empty() {
        eprintln!("no eval cases ran; check target credentials and filters");
        std::process::exit(2)
    }
    if infra == outcomes.len() {
        eprintln!("all executed cases failed at the infrastructure/provider layer");
        std::process::exit(2)
    }
    if outcomes.iter().all(|outcome| {
        outcome.passed
            || (outcome.transcript.error_kind == ErrorKind::Infra
                && outcome.transcript.error.is_some())
    }) {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

async fn run_concurrently(
    filter: Option<String>,
    samples: Option<Vec<String>>,
    concurrency: usize,
) -> (Vec<CaseOutcome>, usize) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut tasks = tokio::task::JoinSet::new();
    let mut order = 0usize;
    let mut skipped = 0usize;
    for eval in mira::registered_evals() {
        let eval = Arc::new(eval);
        let trials = eval.trials.max(1);
        for target in eval.targets.clone() {
            for sample in eval.dataset.samples.clone() {
                for params in eval.axis_combinations() {
                    let key = mira::case_key(&eval.name, &sample.id, &target.label, &params);
                    if filter.as_ref().is_some_and(|value| !key.contains(value))
                        || samples.as_ref().is_some_and(|ids| {
                            !ids.iter().any(|id| mira::glob_match(id, &sample.id))
                        })
                    {
                        continue;
                    }
                    if !target.available {
                        println!("SKIP  {key} (target unavailable)");
                        skipped += trials;
                        continue;
                    }
                    for index in 0..trials {
                        let (eval, sample, target, params) =
                            (eval.clone(), sample.clone(), target.clone(), params.clone());
                        let semaphore = semaphore.clone();
                        let at = order;
                        order += 1;
                        tasks.spawn(async move {
                            let _permit = semaphore.acquire_owned().await.expect("semaphore open");
                            let outcome = mira::runner::run_case(
                                &eval,
                                &sample,
                                &target,
                                &params,
                                Trial {
                                    index,
                                    count: trials,
                                    seed: None,
                                },
                            )
                            .await;
                            eprintln!(
                                "{} {}",
                                if outcome.passed { "pass" } else { "FAIL" },
                                outcome.key()
                            );
                            (at, outcome)
                        });
                    }
                }
            }
        }
    }
    let mut done = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        done.push(joined.expect("case task panicked"));
    }
    done.sort_by_key(|(index, _)| *index);
    (
        done.into_iter().map(|(_, outcome)| outcome).collect(),
        skipped,
    )
}
