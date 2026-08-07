//! Mira eval study: **platform capability**.
//!
//! Measures whether an everruns agent equipped with the `platform`
//! capability turns natural-language requests into the correct platform
//! operations (managing agents, harnesses, apps, channels, sessions) and behaves
//! safely around destructive requests.
//!
//! The dataset is a portable Mira JSONL (`dataset.jsonl`); the subject drives a
//! running everruns server's `platform-chat` session over HTTP. Run with the
//! `mira` host CLI:
//!
//! ```bash
//! mira --bin platform_capability list
//! mira --bin platform_capability run                    # whole matrix
//! mira --bin platform_capability run --tag safety       # subset by tag
//! mira --bin platform_capability run --format html --out report.html
//! ```
//!
//! Configure the target server and model matrix via env (see `subject.rs` and
//! `EVERRUNS_EVAL_TARGETS`).

mod scorers;
mod subject;

use mira::scorer::succeeded;
use mira::{Dataset, Eval, Target, eval};

use crate::scorers::{
    confirmation_boundary, expected_tools, forbidden_tools, platform_commands, response_matches,
    scheduled_agent_state, tool_budget,
};
use crate::subject::EverrunsServerSubject;

/// The dataset travels with the binary so `mira --bin platform_capability` works
/// from any directory.
const DATASET: &str = include_str!("../dataset.jsonl");

/// Model matrix axis. `EVERRUNS_EVAL_TARGETS` is a comma-separated list of
/// everruns model ids; each becomes a matrix target (sent as `model_id` on
/// session creation). Empty → a single `default` target that uses the session's
/// default model, so a bare run works against any configured server.
fn targets() -> Vec<Target> {
    match std::env::var("EVERRUNS_EVAL_TARGETS") {
        Ok(spec) if !spec.trim().is_empty() => spec
            .split(',')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(|model| Target::new(model, "everruns", model))
            .collect(),
        _ => vec![Target::new("default", "everruns", "")],
    }
}

#[eval]
fn platform_capability() -> Eval {
    let dataset = Dataset::from_jsonl_str(DATASET).expect("embedded dataset.jsonl must parse");
    Eval::new("platform_capability")
        .describe("Drive the everruns platform via discover/query/execute from natural language")
        .dataset(dataset)
        .targets(targets())
        .subject(EverrunsServerSubject::from_env())
        // succeeded(): the turn(s) ran without a subject error.
        .scorer(succeeded())
        // expected_tools()/forbidden_tools()/response_matches(): per-sample,
        // driven by each sample's `metadata` (see scorers.rs).
        .scorer(expected_tools())
        .scorer(forbidden_tools())
        .scorer(confirmation_boundary())
        .scorer(platform_commands())
        .scorer(scheduled_agent_state())
        .scorer(tool_budget())
        .scorer(response_matches())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    mira::Study::registered().serve().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_dataset_parses() {
        let ds = Dataset::from_jsonl_str(DATASET).expect("dataset.jsonl parses");
        assert_eq!(ds.samples.len(), 8, "expected 8 focused samples");
        for s in &ds.samples {
            assert!(!s.input.is_empty(), "{} has no input", s.id);
            // Every sample declares at least one expectation the scorers read.
            let has_expectation = s.metadata.contains_key("expect_tools")
                || s.metadata.contains_key("forbid_tools")
                || s.metadata.contains_key("expect_commands")
                || s.metadata.contains_key("expect_scheduled_agent");
            assert!(has_expectation, "{} declares no scorable expectation", s.id);
        }
    }

    #[test]
    fn study_builds() {
        // Exercises the registered eval end to end (dataset + subject + scorers).
        let eval = platform_capability();
        assert_eq!(eval.name, "platform_capability");
    }
}
