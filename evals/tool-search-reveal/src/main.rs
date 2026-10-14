//! Mira eval study: **tool_search reveal modes**.
//!
//! The generic `tool_search` capability can make a found tool callable in two
//! ways. `redefine` (the default) puts the tool's full parameter schema on its
//! definition from the next step; `in_result` keeps the permissive stub in the
//! tool list for good and leaves the model to call from the schema the search
//! result returned. `in_result` never changes the tool list, which keeps the
//! prompt cache warm, but structured tool calling no longer has the real schema
//! to emit against. This study measures what that costs in call quality.
//!
//! Each case runs once per arm (the `reveal` axis) against an in-process
//! Framework agent over 26 fake tools with strict schemas (see
//! `registry.rs`). Scorers grade the tools' own call log.
//!
//! ```bash
//! EVERRUNS_EVAL_TARGETS=openrouter/openai/gpt-6-luna \
//!   doppler run -- ./target/debug/tool_search_reveal --run
//! mira --bin tool_search_reveal run
//! ```

mod registry;
mod scorers;
mod subject;

use std::collections::BTreeMap;
use std::sync::Arc;

use mira::scorer::succeeded;
use mira::{CaseOutcome, Dataset, ErrorKind, Eval, Target, Trial, eval};
use serde_json::{Value, json};

use crate::scorers::{
    FAKE_CALLS, REJECTED, SEARCHES, STUB_BEFORE_SEARCH, calls_to_stub_before_search, correct_call,
    first_try, rejected_calls,
};
use crate::subject::{ARMS, ToolSearchRevealSubject};

const DATASET: &str = include_str!("../dataset.jsonl");

/// `EVERRUNS_EVAL_TARGETS`: comma-separated `provider/model` entries
/// (`anthropic/…`, `openai/…`, `openrouter/<vendor>/<model>`). Unset runs one
/// OpenRouter model, so a single key is enough.
fn targets() -> Vec<Target> {
    let spec = std::env::var("EVERRUNS_EVAL_TARGETS").unwrap_or_default();
    if spec.trim().is_empty() {
        return vec![Target::cloud(
            "openrouter",
            "openai/gpt-6-luna",
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

/// Repetitions per case and arm; one trial is a coin flip (`EVERRUNS_EVAL_TRIALS`).
fn trials() -> usize {
    env_count("EVERRUNS_EVAL_TRIALS").unwrap_or(1)
}

fn env_count(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|count| *count > 0)
}

#[eval]
fn tool_search_reveal() -> Eval {
    let dataset = Dataset::from_jsonl_str(DATASET).expect("embedded dataset.jsonl must parse");
    Eval::new("tool_search_reveal")
        .describe("Call quality of tool_search's `redefine` vs `in_result` reveal modes")
        .dataset(dataset)
        .targets(targets())
        .trials(trials())
        // One arm per value: the same cases, the same tools, only `reveal` differs.
        .axis("reveal", ARMS)
        .subject(ToolSearchRevealSubject)
        .scorer(succeeded())
        .scorer(correct_call())
        .scorer(first_try())
        // Informational, never failing.
        .scorer(rejected_calls())
        .scorer(calls_to_stub_before_search())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // `--run` runs in-process and prints the comparison tables; otherwise serve
    // a Mira host.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--run") {
        return mira::Study::registered().serve().await;
    }
    let value_of = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    let outcomes = run_concurrently(
        value_of("--filter"),
        value_of("--sample").map(|s| s.split(',').map(str::to_string).collect()),
        env_count("EVERRUNS_EVAL_CONCURRENCY").unwrap_or(8),
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
            let value = if score.na {
                "n/a ".to_string()
            } else {
                format!("{:.2}", score.value)
            };
            println!("        {:<28} {value}  {}", score.scorer, score.reason);
        }
        if let Some(error) = &outcome.transcript.error {
            println!("        error: {error}");
        }
    }
    let passed = outcomes.iter().filter(|o| o.passed).count();
    println!(
        "\n{passed} passed, {} failed, {} total",
        outcomes.len() - passed,
        outcomes.len()
    );
    print!("{}", summary_tables(&outcomes));

    if let Some(path) = value_of("--runs-report")
        .or_else(|| std::env::var("EVERRUNS_EVAL_RUNS_REPORT").ok())
        .filter(|path| !path.trim().is_empty())
    {
        let lines: Vec<String> = outcomes.iter().map(|o| run_line(o).to_string()).collect();
        std::fs::write(&path, lines.join("\n") + "\n")?;
        println!("runs report: {path}");
    }

    if passed == outcomes.len() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

/// Mira's in-process `Runner` runs cases one after another, which would take
/// hours over three models; this runs the same cases through
/// `mira::runner::run_case` (so scoring is identical) with a concurrency cap.
async fn run_concurrently(
    filter: Option<String>,
    samples: Option<Vec<String>>,
    concurrency: usize,
) -> Vec<CaseOutcome> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut tasks = tokio::task::JoinSet::new();
    let mut order = 0usize;
    for eval in mira::registered_evals() {
        let eval = Arc::new(eval);
        let trials = eval.trials.max(1);
        for target in eval.targets.clone() {
            for sample in eval.dataset.samples.clone() {
                for params in eval.axis_combinations() {
                    let key = mira::case_key(&eval.name, &sample.id, &target.label, &params);
                    if filter.as_ref().is_some_and(|f| !key.contains(f.as_str()))
                        || samples
                            .as_ref()
                            .is_some_and(|s| !s.iter().any(|p| mira::glob_match(p, &sample.id)))
                    {
                        continue;
                    }
                    if !target.available {
                        println!("SKIP  {key} (no key)");
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
                            let trial = Trial {
                                index,
                                count: trials,
                                seed: None,
                            };
                            let outcome =
                                mira::runner::run_case(&eval, &sample, &target, &params, trial)
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
    done.sort_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, outcome)| outcome).collect()
}

fn arm(outcome: &CaseOutcome) -> &str {
    outcome
        .params
        .get("reveal")
        .map(String::as_str)
        .unwrap_or("?")
}

fn passes(outcome: &CaseOutcome, scorer: &str) -> bool {
    outcome
        .scores
        .iter()
        .any(|s| s.scorer == scorer && s.pass && !s.na)
}

fn metric(outcome: &CaseOutcome, name: &str) -> f64 {
    outcome.transcript.metric(name).unwrap_or(0.0)
}

#[derive(Default)]
struct Tally {
    runs: usize,
    infra: usize,
    correct: usize,
    first: usize,
    rejected: f64,
    stub_before: f64,
    fake_calls: f64,
    searches: f64,
    tool_calls: usize,
    tokens: u64,
    iterations: usize,
}

impl Tally {
    fn add(&mut self, o: &CaseOutcome) {
        // Infra failures (rate limits, outages) say nothing about the arm.
        if o.transcript.error.is_some() && o.transcript.error_kind == ErrorKind::Infra {
            self.infra += 1;
            return;
        }
        self.runs += 1;
        self.correct += passes(o, "correct_call") as usize;
        self.first += passes(o, "first_try") as usize;
        self.rejected += metric(o, REJECTED);
        self.stub_before += metric(o, STUB_BEFORE_SEARCH);
        self.fake_calls += metric(o, FAKE_CALLS);
        self.searches += metric(o, SEARCHES);
        self.tool_calls += o.transcript.tool_calls_count;
        self.tokens += o.transcript.usage.input_tokens + o.transcript.usage.output_tokens;
        self.iterations += o.transcript.iterations;
    }

    fn rate(&self, n: usize) -> String {
        if self.runs == 0 {
            return "-".into();
        }
        format!(
            "{n}/{} {:>3.0}%",
            self.runs,
            100.0 * n as f64 / self.runs as f64
        )
    }

    fn mean(&self, total: f64) -> String {
        if self.runs == 0 {
            return "-".into();
        }
        format!("{:.2}", total / self.runs as f64)
    }
}

/// Per target and arm, then per case and arm (all targets together).
fn summary_tables(outcomes: &[CaseOutcome]) -> String {
    let mut by_target: BTreeMap<(String, String), Tally> = BTreeMap::new();
    let mut by_case: BTreeMap<(String, String), Tally> = BTreeMap::new();
    let mut by_arm: BTreeMap<String, Tally> = BTreeMap::new();
    for o in outcomes {
        by_target
            .entry((o.target.clone(), arm(o).to_string()))
            .or_default()
            .add(o);
        by_case
            .entry((o.sample_id.clone(), arm(o).to_string()))
            .or_default()
            .add(o);
        by_arm.entry(arm(o).to_string()).or_default().add(o);
    }
    let mut out = String::new();
    let header = format!(
        "{:<44} {:<9} {:>12} {:>12} {:>8} {:>8} {:>8} {:>8} {:>10} {:>6} {:>5}\n",
        "target",
        "arm",
        "correct",
        "first_try",
        "rejected",
        "stub<srch",
        "searches",
        "calls",
        "tokens",
        "iters",
        "infra"
    );
    let row = |label: &str, arm: &str, t: &Tally| {
        format!(
            "{:<44} {:<9} {:>12} {:>12} {:>8} {:>8} {:>8} {:>8} {:>10} {:>6} {:>5}\n",
            label,
            arm,
            t.rate(t.correct),
            t.rate(t.first),
            t.mean(t.rejected),
            t.mean(t.stub_before),
            t.mean(t.searches),
            t.mean(t.tool_calls as f64),
            t.mean(t.tokens as f64),
            t.mean(t.iterations as f64),
            t.infra
        )
    };
    out.push_str("\nPer target and arm (rejected, stub<srch, searches, calls, tokens, iters are means per run)\n");
    out.push_str(&header);
    for ((target, arm), t) in &by_target {
        out.push_str(&row(target, arm, t));
    }
    for (arm, t) in &by_arm {
        out.push_str(&row("ALL", arm, t));
    }
    out.push_str("\nPer case and arm (all targets)\n");
    out.push_str(&header.replacen("target", "case  ", 1));
    for ((case, arm), t) in &by_case {
        out.push_str(&row(case, arm, t));
    }
    let totals: BTreeMap<&String, (f64, u64)> = by_arm
        .iter()
        .map(|(arm, t)| (arm, (t.rejected, t.tokens)))
        .collect();
    out.push_str(&format!(
        "\nTotals (rejected calls, tokens) per arm: {totals:?}\n"
    ));
    out
}

/// One JSON line per run, for reading failures after the fact.
fn run_line(o: &CaseOutcome) -> Value {
    json!({
        "case": o.sample_id,
        "target": o.target,
        "reveal": arm(o),
        "trial": o.trial.index,
        "passed": o.passed,
        "correct_call": passes(o, "correct_call"),
        "first_try": passes(o, "first_try"),
        "scores": o.scores.iter().map(|s| json!({"scorer": s.scorer, "pass": s.pass, "na": s.na, "reason": s.reason})).collect::<Vec<_>>(),
        "calls": o.transcript.metadata.get(subject::CALLS_KEY),
        "tool_calls": o.transcript.tool_calls,
        "metrics": o.transcript.metrics,
        "input_tokens": o.transcript.usage.input_tokens,
        "output_tokens": o.transcript.usage.output_tokens,
        "iterations": o.transcript.iterations,
        "error": o.transcript.error,
        "final_response": o.transcript.final_response,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::{
        ChatDriver, LlmCallConfig, LlmResponseStream, Provider, ProviderEndpoint, ToolCall,
        ToolDefinition,
    };
    use everruns_llmsim::{LlmSimConfig, LlmSimDriver};
    use std::sync::Mutex;

    #[test]
    fn dataset_is_well_formed_and_its_expectations_are_reachable() {
        let ds = Dataset::from_jsonl_str(DATASET).expect("dataset.jsonl parses");
        assert!((6..=10).contains(&ds.samples.len()), "{}", ds.samples.len());
        let names = registry::names();
        assert!(
            names.len() >= 25,
            "a catalogue worth searching: {}",
            names.len()
        );
        let mut seen = std::collections::BTreeSet::new();
        for s in &ds.samples {
            assert!(seen.insert(s.id.clone()), "duplicate id {}", s.id);
            let expect = s.metadata["expect_calls"].as_array().expect("expect_calls");
            for entry in expect {
                let tool = entry["tool"].as_str().unwrap();
                let schema = registry::schema_of(tool)
                    .unwrap_or_else(|| panic!("{}: `{tool}` is not in the registry", s.id));
                // An expected input that its own tool would refuse is a broken case.
                if let Some(input) = entry.get("input") {
                    let errors: Vec<String> = jsonschema::validator_for(&schema)
                        .unwrap()
                        .iter_errors(input)
                        .map(|e| e.to_string())
                        // Free-text fields are checked by `input_regex`, not `input`.
                        .filter(|e| {
                            !(entry.get("input_regex").is_some() && e.contains("required property"))
                        })
                        .collect();
                    assert!(errors.is_empty(), "{}: {tool}: {errors:?}", s.id);
                }
                for pattern in entry
                    .get("input_regex")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|m| m.values())
                {
                    regex::Regex::new(pattern.as_str().unwrap()).expect("input_regex compiles");
                }
            }
            for tool in s
                .metadata
                .get("forbid_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                assert!(names.contains(&tool.as_str().unwrap()), "{}: {tool}", s.id);
            }
        }
    }

    /// llmsim behind a wrapper that records the tool list of every request.
    struct Recording {
        inner: LlmSimDriver,
        tools: Arc<Mutex<Vec<Vec<ToolDefinition>>>>,
    }

    #[async_trait::async_trait]
    impl ChatDriver for Recording {
        async fn chat_completion_stream(
            &self,
            endpoint: &ProviderEndpoint,
            messages: Vec<everruns::llm::Message>,
            config: &LlmCallConfig,
        ) -> Result<LlmResponseStream, everruns::AgentLoopError> {
            self.tools.lock().unwrap().push(config.tools.clone());
            self.inner
                .chat_completion_stream(endpoint, messages, config)
                .await
        }
    }

    fn call(id: &str, name: &str, arguments: Value) -> Vec<ToolCall> {
        vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments,
        }]
    }

    const REFUND: &str = "billing_issue_refund";

    /// One scripted run per arm: search, a refused call (amount as a decimal
    /// string, an invented reason), then the right call. Proves the harness
    /// wires `reveal` through to the capability, that the fake tool's own
    /// validation is the gate, and how each arm's tool list differs.
    async fn scripted(reveal: &str) -> (Transcript, Vec<Vec<ToolDefinition>>) {
        let config = LlmSimConfig::fixed("Refunded.").with_tool_call_sequence(vec![
            call("c1", "tool_search", json!({"query": "refund invoice"})),
            call(
                "c2",
                REFUND,
                json!({"invoice_id": "inv_9001", "amount_cents": "25.50", "reason": "double_charge"}),
            ),
            call(
                "c3",
                REFUND,
                json!({"invoice_id": "inv_9001", "amount_cents": 2550, "reason": "duplicate", "notify_customer": false}),
            ),
            vec![],
        ]);
        let tools = Arc::new(Mutex::new(Vec::new()));
        let driver = Recording {
            inner: LlmSimDriver::new(config),
            tools: tools.clone(),
        };
        let sample = refund_sample();
        let log = registry::CallLog::default();
        let session = subject::build_session(
            &sample,
            &Target::new("offline", "offline", "test-model"),
            Provider::new("offline", driver),
            &log,
            reveal,
        )
        .expect("agent builds with tool_search");
        let mut transcript = subject::run_turns(&session, &sample).await;
        subject::finish(&mut transcript, &log);
        assert!(transcript.error.is_none(), "{:?}", transcript.error);
        let tools = tools.lock().unwrap().clone();
        (transcript, tools)
    }

    use mira::Transcript;

    fn refund_sample() -> mira::Sample {
        serde_json::from_value(json!({
            "id": "scripted",
            "input": ["Refund $25.50 on inv_9001, charged twice, no notification."],
            "metadata": {"expect_calls": [{
                "tool": REFUND,
                "input": {"invoice_id": "inv_9001", "amount_cents": 2550, "reason": "duplicate", "notify_customer": false}
            }]}
        }))
        .unwrap()
    }

    fn params_of(tools: &[ToolDefinition], name: &str) -> Value {
        tools
            .iter()
            .find(|t| t.name() == name)
            .unwrap_or_else(|| panic!("{name} missing from the request's tools"))
            .parameters()
            .clone()
    }

    fn stub() -> Value {
        json!({"type": "object", "additionalProperties": true})
    }

    async fn check_common(transcript: &Transcript) {
        // The engine refused the first call against the tool's real schema,
        // in both arms, so it never reached the handler.
        let made = scorers::calls(transcript);
        assert_eq!(made.len(), 2, "{made:?}");
        assert_eq!(made[0]["valid"], false);
        assert_eq!(made[0]["rejected_by"], "engine");
        assert_eq!(made[0]["input"]["amount_cents"], "25.50");
        let errors = made[0]["errors"].to_string();
        assert!(errors.contains("/amount_cents"), "{errors}");
        assert!(errors.contains("notify_customer"), "{errors}");
        assert_eq!(made[1]["valid"], true);
        let handled = transcript.metadata[subject::TOOL_LOG_KEY]
            .as_array()
            .unwrap();
        assert_eq!(handled.len(), 1, "{handled:?}");
        assert_eq!(transcript.metric(scorers::REJECTED), Some(1.0));
        assert_eq!(transcript.metric(scorers::STUB_BEFORE_SEARCH), Some(0.0));
        assert_eq!(transcript.metric(scorers::SEARCHES), Some(1.0));
        let sample = refund_sample();
        assert!(correct_call().score(&sample, transcript).await.pass);
        assert!(!first_try().score(&sample, transcript).await.pass);
        // The model saw the refusal and why.
        let refusal = transcript
            .events
            .iter()
            .filter(|e| e["type"] == "tool.completed" && e["data"]["tool_name"] == REFUND)
            .map(|e| e["data"].to_string())
            .next()
            .unwrap();
        assert!(refusal.contains("invalid_tool_arguments"), "{refusal}");
    }

    #[tokio::test]
    async fn in_result_keeps_the_stub_until_invalid_call_recovery() {
        let (transcript, requests) = scripted("in_result").await;
        check_common(&transcript).await;
        assert!(requests.len() >= 3, "{}", requests.len());
        // Search alone preserves the stub. The engine's invalid-call recovery
        // reveals the rejected tool's schema on the following request.
        for (step, tools) in requests[..2].iter().enumerate() {
            assert_eq!(params_of(tools, REFUND), stub(), "step {step}");
        }
        for (step, tools) in requests[2..].iter().enumerate() {
            assert_eq!(
                params_of(tools, REFUND),
                registry::schema_of(REFUND).unwrap(),
                "recovery step {step}"
            );
        }
        for tools in &requests {
            assert_eq!(params_of(tools, "inventory_reserve_stock"), stub());
        }
        // The search result itself carried the full schema.
        let search = transcript
            .events
            .iter()
            .find(|e| e["type"] == "tool.completed" && e["data"]["tool_name"] == "tool_search")
            .unwrap()["data"]
            .to_string();
        assert!(search.contains("notify_customer"), "{search}");
        assert!(search.contains("placeholder schema"), "{search}");
    }

    #[tokio::test]
    async fn redefine_replaces_the_stub_after_the_search() {
        let (transcript, requests) = scripted("redefine").await;
        check_common(&transcript).await;
        assert_eq!(
            params_of(&requests[0], REFUND),
            stub(),
            "deferred before the search"
        );
        let last = requests.last().unwrap();
        assert_eq!(
            params_of(last, REFUND),
            registry::schema_of(REFUND).unwrap(),
            "full schema on the definition after the search"
        );
        // Tools never searched for stay deferred.
        assert_eq!(params_of(last, "inventory_reserve_stock"), stub());
    }

    #[test]
    fn study_builds_with_both_arms() {
        let eval = tool_search_reveal();
        assert_eq!(eval.name, "tool_search_reveal");
        assert_eq!(eval.axis_combinations().len(), 2);
    }
}
