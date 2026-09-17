//! Mira study: **calibration** of the model-backed guardrail engines.
//!
//! Unit tests prove a guardrail's plumbing — that a block blocks and a failure
//! fails open. They say nothing about the number that actually decides whether
//! a guardrail is usable: how much of what should be blocked is blocked, and
//! how much ordinary work is blocked along with it.
//!
//! This study measures that against a labeled corpus, across both engines and
//! several thresholds, through the shipped decision path. The output is the
//! table you pick a threshold from, and the only honest way to answer "is `jev`
//! better than `utility_llm` here".
//!
//! ```sh
//! UTILITY_TYPESAFE_API_KEY=... UTILITY_OPENAI_API_KEY=... \
//!   EVERRUNS_GUARDRAIL_ENGINES=jev,utility_llm \
//!   EVERRUNS_GUARDRAIL_THRESHOLDS=30,50,70 \
//!   mira --bin guardrail_calibration
//! ```
//!
//! There is no model target axis: the engines are deployment-owned services with
//! fixed models, which is exactly why their behavior is worth pinning down.

mod scorers;
mod subject;

use mira::{Dataset, Eval, Target, eval};

use crate::scorers::{allows_benign, blocks_violations};
use crate::subject::GuardrailCalibrationSubject;

/// The dataset travels with the binary so the study runs from any directory.
const DATASET: &str = include_str!("../dataset.jsonl");

/// A comma-separated axis from the environment, with a default.
fn axis_values(env: &str, default: &str) -> Vec<String> {
    let raw = std::env::var(env).unwrap_or_default();
    let values: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect();
    if values.is_empty() {
        vec![default.to_string()]
    } else {
        values
    }
}

#[eval]
fn guardrail_calibration() -> Eval {
    let dataset = Dataset::from_jsonl_str(DATASET).expect("embedded dataset.jsonl must parse");
    Eval::new("guardrail-calibration")
        .describe("Block rate and false-positive rate for the model-backed guardrail engines")
        .dataset(dataset)
        // The engine under test is the subject, not a model provider, so the
        // target axis carries a single placeholder.
        .targets(vec![Target::new("engine", "engine", "deployment-owned")])
        .axis(
            "engine",
            axis_values("EVERRUNS_GUARDRAIL_ENGINES", subject::DEFAULT_ENGINE),
        )
        .axis(
            "threshold",
            axis_values("EVERRUNS_GUARDRAIL_THRESHOLDS", subject::DEFAULT_THRESHOLD),
        )
        .subject(GuardrailCalibrationSubject)
        .scorer(blocks_violations())
        .scorer(allows_benign())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    mira::Study::registered().serve().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataset() -> Dataset {
        Dataset::from_jsonl_str(DATASET).expect("dataset parses")
    }

    #[test]
    fn every_case_is_labeled_and_carries_a_policy() {
        for sample in &dataset().samples {
            assert!(
                sample
                    .metadata
                    .get("should_block")
                    .and_then(|v| v.as_bool())
                    .is_some(),
                "{}: calibration needs a ground-truth label",
                sample.id
            );
            assert!(
                sample
                    .metadata
                    .get("policy")
                    .and_then(|v| v.as_str())
                    .is_some_and(|p| !p.is_empty()),
                "{}: a judge case needs the policy it is judged against",
                sample.id
            );
        }
    }

    #[test]
    fn the_corpus_is_balanced_enough_to_read() {
        // A corpus of one class measures nothing: all-violating cases make a
        // block-everything guardrail look perfect, and vice versa.
        let samples = dataset().samples;
        let blocking = samples
            .iter()
            .filter(|s| crate::scorers::should_block(s))
            .count();
        let benign = samples.len() - blocking;
        assert!(
            blocking >= 4 && benign >= 4,
            "need both classes represented: {blocking} violating, {benign} benign"
        );
    }

    /// Runs the whole corpus through both engines and prints the calibration
    /// table. Not a pass/fail assertion about model quality — it is the
    /// measurement itself, and it needs both credentials to mean anything.
    ///
    /// `cargo test -- --nocapture calibration_table`
    #[tokio::test]
    async fn calibration_table() {
        use crate::scorers::should_block;
        use crate::subject::{BLOCKED_KEY, GuardrailCalibrationSubject, SKIPPED_KEY};
        use mira::{RunCx, Subject};

        let engines: Vec<&str> = ["jev", "utility_llm"]
            .into_iter()
            .filter(|engine| {
                let var = if *engine == "jev" {
                    "UTILITY_TYPESAFE_API_KEY"
                } else {
                    "UTILITY_OPENAI_API_KEY"
                };
                std::env::var(var).is_ok_and(|k| !k.trim().is_empty())
            })
            .collect();
        if engines.is_empty() {
            eprintln!("skipping: no engine credentials configured");
            return;
        }

        let samples = dataset().samples;
        // The same sweep the study's threshold axis performs, so the table can
        // be reproduced without the Mira host.
        let thresholds = axis_values("EVERRUNS_GUARDRAIL_THRESHOLDS", "50");
        for engine in engines {
            for threshold in &thresholds {
                let (mut caught, mut violations, mut allowed, mut benign) = (0, 0, 0, 0);
                let mut misses = Vec::new();
                for sample in &samples {
                    let mut cx = RunCx::new(Target::new("engine", "engine", "deployment-owned"));
                    cx.params.insert("engine".into(), engine.into());
                    cx.params.insert("threshold".into(), threshold.clone());
                    let t = GuardrailCalibrationSubject.run(sample, &cx).await;
                    if t.metadata.contains_key(SKIPPED_KEY) {
                        continue;
                    }
                    let blocked = t.metadata[BLOCKED_KEY].as_bool().expect("a decision");
                    if should_block(sample) {
                        violations += 1;
                        if blocked {
                            caught += 1;
                        } else {
                            misses.push(format!("missed {}", sample.id));
                        }
                    } else {
                        benign += 1;
                        if blocked {
                            misses.push(format!("false positive {}", sample.id));
                        } else {
                            allowed += 1;
                        }
                    }
                }
                println!(
                    "{engine:<12} t={threshold:<3} caught {caught}/{violations} violations, \
                 allowed {allowed}/{benign} benign"
                );
                for miss in &misses {
                    println!("                    {miss}");
                }
                assert!(violations > 0 && benign > 0, "{engine}: nothing measured");
            }
        }
    }

    #[test]
    fn near_misses_are_represented() {
        // Cases that only a calibrated engine gets right are the reason this
        // study exists; a corpus of obvious cases separates nothing.
        let count = dataset()
            .samples
            .iter()
            .filter(|s| s.tags.iter().any(|t| t == "near-miss"))
            .count();
        assert!(count >= 3, "only {count} near-miss case(s)");
    }
}
