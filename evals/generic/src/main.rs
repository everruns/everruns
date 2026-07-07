//! Mira eval study: **generic**.
//!
//! Generic agent cases — instruction following, structured output, reasoning,
//! extraction, multi-turn state, file/shell/time tool use, and tool safety —
//! run **directly on the local `everruns-runtime`** (in-process; no server, no
//! HTTP, no database). Built for regression testing the working tree and for
//! comparing models, reasoning efforts, harness profiles, and runtime
//! configurations on the same dataset.
//!
//! The matrix:
//! - **target** (model): key-gated `anthropic`/`openai`/`openrouter` defaults,
//!   or `EVERRUNS_EVAL_TARGETS="anthropic/claude-sonnet-5,openai/gpt-5.5,openrouter/z-ai/glm-5.2"`
//! - **effort** axis: `EVERRUNS_EVAL_EFFORTS="default,low,high"`
//! - **harness** axis: `EVERRUNS_EVAL_HARNESSES="minimal,workspace,coding"`
//! - **config** axis: `EVERRUNS_EVAL_CONFIGS="default,tight-iterations"`
//!
//! Run with the `mira` host CLI:
//!
//! ```bash
//! mira --bin generic_evals list
//! mira --bin generic_evals run                    # whole matrix
//! mira --bin generic_evals run --preset smoke     # quick text-only subset
//! mira --bin generic_evals run --format html --out report.html
//! ```

mod profiles;
mod scorers;
mod subject;

use mira::{Dataset, Eval, Target, eval};

use crate::scorers::{
    expected_tools, file_expectations, forbidden_tools, response_avoids, response_matches,
    tool_call_budget, turn_completed,
};
use crate::subject::GenericRuntimeSubject;

/// The dataset travels with the binary so `mira --bin generic_evals` works
/// from any directory.
const DATASET: &str = include_str!("../dataset.jsonl");

/// Default model matrix: one key-gated model per supported provider — without
/// keys those cases are skipped, so a key-free run stays green. GLM rides
/// through OpenRouter (its slug keeps the vendor prefix).
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5";
const DEFAULT_OPENAI_MODEL: &str = "gpt-5.5";
const DEFAULT_OPENROUTER_MODEL: &str = "z-ai/glm-5.2";

/// Model matrix axis. `EVERRUNS_EVAL_TARGETS` is a comma-separated list of
/// `provider/model` entries (`anthropic/...`, `openai/...`,
/// `openrouter/<vendor>/<model>`). Unset → key-gated provider defaults.
fn targets() -> Vec<Target> {
    let spec = std::env::var("EVERRUNS_EVAL_TARGETS").unwrap_or_default();
    if spec.trim().is_empty() {
        return vec![
            Target::anthropic(DEFAULT_ANTHROPIC_MODEL),
            Target::openai(DEFAULT_OPENAI_MODEL),
            Target::cloud("openrouter", DEFAULT_OPENROUTER_MODEL, "OPENROUTER_API_KEY"),
        ];
    }
    spec.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.split_once('/') {
            Some(("anthropic", model)) => Target::anthropic(model),
            Some(("openai", model)) => Target::openai(model),
            Some(("openrouter", model)) => Target::cloud("openrouter", model, "OPENROUTER_API_KEY"),
            // Unknown providers stay in the matrix; the subject rejects them
            // with a clear infra error naming the supported set.
            Some((provider, model)) => Target::new(entry, provider, model),
            None => Target::new(entry, entry, ""),
        })
        .collect()
}

/// One extra matrix axis from a comma-separated env var, with a default.
fn axis_values(env: &str, default: &str) -> Vec<String> {
    let spec = std::env::var(env).unwrap_or_default();
    let values: Vec<String> = spec
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect();
    if values.is_empty() {
        vec![default.to_string()]
    } else {
        values
    }
}

#[eval]
fn generic() -> Eval {
    let dataset = Dataset::from_jsonl_str(DATASET).expect("embedded dataset.jsonl must parse");
    Eval::new("generic")
        .describe("Generic agent cases run in-process on the local everruns-runtime")
        .dataset(dataset)
        .targets(targets())
        .axis(
            "effort",
            axis_values("EVERRUNS_EVAL_EFFORTS", profiles::DEFAULT_EFFORT),
        )
        .axis(
            "harness",
            axis_values("EVERRUNS_EVAL_HARNESSES", profiles::DEFAULT_HARNESS),
        )
        .axis(
            "config",
            axis_values("EVERRUNS_EVAL_CONFIGS", profiles::DEFAULT_CONFIG),
        )
        .subject(GenericRuntimeSubject)
        // turn_completed(): the turn(s) ran without a subject error.
        .scorer(turn_completed())
        // The rest are per-sample, driven by each sample's `metadata`
        // (see scorers.rs for the schema).
        .scorer(expected_tools())
        .scorer(forbidden_tools())
        .scorer(response_matches())
        .scorer(response_avoids())
        .scorer(file_expectations())
        .scorer(tool_call_budget())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    mira::Study::registered().serve().await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Metadata keys a scorer reads — every sample must declare at least one so
    /// no case silently free-rides on `turn_completed` alone.
    const EXPECTATION_KEYS: &[&str] = &[
        "expect_tools",
        "forbid_tools",
        "expect_regex",
        "forbid_regex",
        "expect_files",
        "min_tool_calls",
        "max_tool_calls",
    ];

    #[test]
    fn embedded_dataset_parses_and_declares_expectations() {
        let ds = Dataset::from_jsonl_str(DATASET).expect("dataset.jsonl parses");
        assert!(ds.samples.len() >= 30, "expected a substantial dataset");
        let mut seen = std::collections::HashSet::new();
        for s in &ds.samples {
            assert!(seen.insert(s.id.clone()), "duplicate sample id {}", s.id);
            assert!(!s.input.is_empty(), "{} has no input", s.id);
            assert!(!s.tags.is_empty(), "{} has no tags", s.id);
            assert!(
                EXPECTATION_KEYS.iter().any(|k| s.metadata.contains_key(*k)),
                "{} declares no scorable expectation",
                s.id
            );
        }
    }

    #[test]
    fn requires_metadata_names_known_capabilities() {
        // Every `requires` entry must be provided by at least one harness
        // profile, otherwise the case is unrunnable on every axis value.
        let ds = Dataset::from_jsonl_str(DATASET).unwrap();
        let known: std::collections::HashSet<&str> = profiles::HARNESS_PROFILES
            .iter()
            .flat_map(|p| p.capabilities.iter().copied())
            .collect();
        for s in &ds.samples {
            let Some(requires) = s.metadata.get("requires") else {
                continue;
            };
            for cap in requires.as_array().expect("requires is a list") {
                let cap = cap.as_str().expect("requires entries are strings");
                assert!(
                    known.contains(cap),
                    "{}: unknown capability '{}'",
                    s.id,
                    cap
                );
            }
        }
    }

    #[test]
    fn dataset_regexes_compile() {
        let ds = Dataset::from_jsonl_str(DATASET).unwrap();
        for s in &ds.samples {
            for key in ["expect_regex", "forbid_regex"] {
                let Some(value) = s.metadata.get(key) else {
                    continue;
                };
                let patterns: Vec<&str> = match value {
                    serde_json::Value::String(p) => vec![p.as_str()],
                    serde_json::Value::Array(items) => {
                        items.iter().filter_map(|v| v.as_str()).collect()
                    }
                    _ => panic!("{}: {key} must be a string or list", s.id),
                };
                for p in patterns {
                    regex::Regex::new(p)
                        .unwrap_or_else(|e| panic!("{}: bad {key} /{p}/: {e}", s.id));
                }
            }
            if let Some(files) = s.metadata.get("expect_files") {
                for entry in files.as_array().expect("expect_files is a list") {
                    assert!(
                        entry.get("path").and_then(|p| p.as_str()).is_some(),
                        "{}: expect_files entry lacks a path",
                        s.id
                    );
                    if let Some(p) = entry.get("regex").and_then(|r| r.as_str()) {
                        regex::Regex::new(p)
                            .unwrap_or_else(|e| panic!("{}: bad file regex /{p}/: {e}", s.id));
                    }
                }
            }
        }
    }

    #[test]
    fn study_builds_with_full_default_matrix() {
        let eval = generic();
        assert_eq!(eval.name, "generic");
        assert_eq!(
            eval.targets.len(),
            3,
            "anthropic + openai + openrouter defaults"
        );
        // effort × harness × config default to a single combination.
        assert_eq!(eval.axis_combinations().len(), 1);
    }

    #[test]
    fn attachments_are_self_contained_images() {
        // Vision cases must carry their pixels inline (base64), not reference
        // external URLs — the dataset has no network dependencies.
        let ds = Dataset::from_jsonl_str(DATASET).unwrap();
        let mut vision_cases = 0;
        for s in &ds.samples {
            for part in &s.attachments {
                match part {
                    mira::Part::Image { source, .. } => {
                        assert!(
                            source.data().is_some(),
                            "{}: image attachment must be inline base64",
                            s.id
                        );
                        vision_cases += 1;
                    }
                    other => panic!("{}: unsupported attachment {other:?}", s.id),
                }
            }
        }
        assert!(
            vision_cases >= 3,
            "expected multimodal cases in the dataset"
        );
    }

    #[test]
    fn tool_cases_declare_requires() {
        // A case that expects tool usage must also say which capability
        // provides the tool, so non-tool harness profiles skip it cleanly.
        let ds = Dataset::from_jsonl_str(DATASET).unwrap();
        for s in &ds.samples {
            let expects_tools = s.metadata.contains_key("expect_tools")
                || s.metadata.contains_key("expect_files")
                || s.metadata.contains_key("min_tool_calls");
            if expects_tools {
                assert!(
                    s.metadata.contains_key("requires"),
                    "{} expects tool usage but declares no `requires`",
                    s.id
                );
            }
        }
    }
}
