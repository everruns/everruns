//! Mira eval study: **tools in shell**.
//!
//! Measures whether a model finds and calls a tool it cannot see, through the
//! shell's `tools --help` and `tools search`, and how many calls that costs.
//! Each case runs an in-process Framework agent with `bashkit_shell` and
//! `tools_in_shell` over a small fake registry (see `registry.rs`), so a run
//! needs one model key and nothing else.
//!
//! ```bash
//! EVERRUNS_EVAL_TARGETS=openrouter/openai/gpt-5.5 \
//!   doppler run --command './target/debug/tools_in_shell --run'
//! mira --bin tools_in_shell run
//! ```

// The friction classifier is the platform-capability study's, included by
// path so both studies count help reads, rejections and real calls the same
// way. It depends on serde_json and regex only, not on a Mira version.
#[path = "../../platform-capability/src/friction.rs"]
mod friction;
mod registry;
mod scorers;
mod subject;

use mira::scorer::succeeded;
use mira::{Dataset, Eval, Target, eval};

use crate::friction::FrictionRun;
use crate::scorers::{
    expected_calls, forbidden_calls, response_matches, tool_budget, tools_friction,
};
use crate::subject::ToolsInShellSubject;

const DATASET: &str = include_str!("../dataset.jsonl");

/// `EVERRUNS_EVAL_TARGETS`: comma-separated `provider/model` entries
/// (`anthropic/…`, `openai/…`, `openrouter/<vendor>/<model>`). Unset runs one
/// OpenRouter model, so a single key is enough.
fn targets() -> Vec<Target> {
    let spec = std::env::var("EVERRUNS_EVAL_TARGETS").unwrap_or_default();
    if spec.trim().is_empty() {
        return vec![Target::cloud(
            "openrouter",
            "openai/gpt-5.5",
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

/// Repetitions per case; one trial is a coin flip (`EVERRUNS_EVAL_TRIALS`).
fn trials() -> usize {
    std::env::var("EVERRUNS_EVAL_TRIALS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|count| *count > 0)
        .unwrap_or(1)
}

#[eval]
fn tools_in_shell() -> Eval {
    let dataset = Dataset::from_jsonl_str(DATASET).expect("embedded dataset.jsonl must parse");
    Eval::new("tools_in_shell")
        .describe("Find and call hidden tools through the shell's `tools` command")
        .dataset(dataset)
        .targets(targets())
        .trials(trials())
        .subject(ToolsInShellSubject)
        .scorer(succeeded())
        .scorer(expected_calls())
        .scorer(forbidden_calls())
        .scorer(tool_budget())
        // Informational, never failing: help reads, rejections, real calls.
        .scorer(tools_friction())
        .scorer(response_matches())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // `--run` runs in-process and prints a report plus the friction table, as
    // the platform-capability study does; otherwise serve a Mira host.
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

    let report = mira::Runner::new()
        .extend(mira::registered_evals())
        .tag(value_of("--tag"))
        .filter(value_of("--filter"))
        .samples(value_of("--sample").map(|s| s.split(',').map(str::to_string).collect()))
        .run()
        .await;

    for outcome in &report.outcomes {
        println!(
            "{}  {}",
            if outcome.passed { "PASS" } else { "FAIL" },
            outcome.key()
        );
        for score in &outcome.scores {
            if score.na && outcome.passed {
                continue;
            }
            let value = if score.na {
                "n/a ".to_string()
            } else {
                format!("{:.2}", score.value)
            };
            println!("        {:<20} {value}  {}", score.scorer, score.reason);
        }
        if let Some(error) = &outcome.transcript.error {
            println!("        error: {error}");
        }
    }
    println!(
        "\n{} passed, {} failed, {} total",
        report.passed(),
        report.failed(),
        report.total()
    );

    let runs: Vec<FrictionRun> = report.outcomes.iter().map(friction_run).collect();
    print!("{}", friction::summary_table(&runs));
    if let Some(path) = value_of("--friction-report")
        .or_else(|| std::env::var("EVERRUNS_EVAL_FRICTION_REPORT").ok())
        .filter(|path| !path.trim().is_empty())
    {
        std::fs::write(&path, friction::report_jsonl(&runs, "tools"))?;
        println!("friction report: {path}");
    }

    if report.all_passed() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn friction_run(outcome: &mira::CaseOutcome) -> FrictionRun {
    FrictionRun {
        case: outcome.sample_id.clone(),
        target: outcome.target.clone(),
        trial: outcome.trial.index,
        passed: outcome.passed,
        tool_calls: outcome.transcript.tool_calls_count,
        error: outcome.transcript.error.clone(),
        friction: scorers::friction_of(&outcome.transcript),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::{Provider, ToolCall};
    use everruns_llmsim::{LlmSimConfig, LlmSimDriver};
    use serde_json::{Value, json};

    #[test]
    fn dataset_is_well_formed_and_names_real_tools() {
        let ds = Dataset::from_jsonl_str(DATASET).expect("dataset.jsonl parses");
        assert!(
            (4..=6).contains(&ds.samples.len()),
            "a focused slice: {}",
            ds.samples.len()
        );
        let names = registry::names();
        let mut seen = std::collections::BTreeSet::new();
        for s in &ds.samples {
            assert!(seen.insert(s.id.clone()), "duplicate id {}", s.id);
            assert!(!s.tags.is_empty(), "{} has no tags", s.id);
            let expect = s.metadata["expect_calls"]
                .as_array()
                .unwrap_or_else(|| panic!("{} expects no call", s.id));
            let forbid = s
                .metadata
                .get("forbid_calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for tool in expect
                .iter()
                .map(|e| e["tool"].as_str().unwrap_or_default())
                .chain(forbid.iter().map(|t| t.as_str().unwrap_or_default()))
            {
                assert!(
                    names.contains(&tool),
                    "{}: `{tool}` is not in the registry",
                    s.id
                );
            }
            for entry in expect {
                for pattern in entry
                    .get("input_regex")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|m| m.values())
                {
                    regex::Regex::new(pattern.as_str().unwrap()).expect("input_regex compiles");
                }
            }
            if let Some(pattern) = s.metadata.get("expect_regex").and_then(Value::as_str) {
                regex::Regex::new(pattern).expect("expect_regex compiles");
            }
        }
    }

    fn bash(id: &str, script: &str) -> Vec<ToolCall> {
        vec![ToolCall {
            id: id.into(),
            name: "bash".into(),
            arguments: json!({ "commands": script }),
        }]
    }

    /// The whole path with a scripted model and no key: the shipped
    /// capability hides the registry, the builtin answers help and search,
    /// refuses a wrong name and a malformed input with its own envelopes, runs
    /// the right call, and the friction count and call log read all of it.
    #[tokio::test]
    async fn scripted_run_exercises_help_search_refusals_and_a_real_call() {
        let config = LlmSimConfig::fixed("Ticket TCK-1042 is open.").with_tool_call_sequence(vec![
            bash("c1", "tools --help"),
            bash("c2", "tools search ticket"),
            bash("c3", "tools tickets open-ticket '{\"title\":\"Checkout fails on Safari\"}'"),
            bash(
                "c4",
                "tools tickets create-ticket '{\"title\":\"Checkout fails on Safari\",\"priority\":\"urgent\"}'",
            ),
            bash(
                "c5",
                "tools tickets create-ticket --help\ntools tickets create-ticket <<'JSON'\n{\"title\":\"Checkout fails on Safari\",\"priority\":\"high\",\"labels\":[\"checkout\",\"safari\"]}\nJSON",
            ),
            vec![],
        ]);
        let sample: mira::Sample = serde_json::from_value(json!({
            "id": "scripted",
            "input": ["Open a high-priority ticket."],
            "metadata": {"expect_calls": [{
                "tool": "mcp_tickets__create_ticket",
                "input": {"title": "Checkout fails on Safari", "priority": "high", "labels": ["checkout", "safari"]},
                "max": 1
            }]}
        }))
        .unwrap();
        let log = registry::CallLog::default();
        let session = subject::build_session(
            &sample,
            &Target::new("offline", "offline", "test-model"),
            Provider::new("offline", LlmSimDriver::new(config)),
            &log,
        )
        .expect("agent builds with the shipped capabilities");
        let mut transcript = subject::run_turns(&session, &sample).await;
        subject::finish(&mut transcript, &log);
        assert!(transcript.error.is_none(), "{:?}", transcript.error);
        assert_eq!(transcript.tool_calls, ["bash"; 5], "only bash is direct");

        let outputs: Vec<String> = transcript
            .events
            .iter()
            .filter(|e| e["type"] == "tool.completed")
            .map(|e| e["data"].to_string())
            .collect();
        // Not `tickets`: the bash tool's default `auto` output keeps the head
        // and tail of a long success, and the source list sits in the middle
        // (see the README's "What the first runs should watch").
        assert!(
            outputs[0].contains("weather-forecast"),
            "root help lists tools: {}",
            outputs[0]
        );
        assert!(
            outputs[1].contains("create-ticket"),
            "search finds the tool: {}",
            outputs[1]
        );

        let friction = scorers::friction_of(&transcript);
        let classes: Vec<&str> = friction.calls.iter().map(|c| c.class.name()).collect();
        assert_eq!(
            classes,
            ["help", "help", "rejected", "rejected", "real"],
            "{:#?}",
            friction.calls
        );
        assert!(
            friction.calls[2]
                .error
                .as_deref()
                .is_some_and(|e| e.contains("unknown_command")),
            "{:?}",
            friction.calls[2]
        );
        assert!(
            friction.calls[3]
                .error
                .as_deref()
                .is_some_and(|e| e.contains("invalid_input")),
            "{:?}",
            friction.calls[3]
        );
        assert_eq!(transcript.metric("friction.rejected"), Some(2.0));

        // Refused input never reached the tool: exactly one call ran.
        let made = transcript.metadata[subject::CALLS_KEY].as_array().unwrap();
        assert_eq!(made.len(), 1, "{made:?}");
        assert!(expected_calls().score(&sample, &transcript).await.pass);

        let report = friction.report("tools");
        assert_eq!(
            report["help_pages"],
            json!([
                "tools",
                "tools search ticket",
                "tools tickets create-ticket"
            ])
        );
    }

    #[test]
    fn study_builds() {
        assert_eq!(tools_in_shell().name, "tools_in_shell");
    }
}
