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
        // Not an exact count. This asserted 9 against a 13-sample dataset for
        // long enough to go unnoticed, because nothing ran it: a number that
        // has to be edited with every added case is a number that gets edited
        // without being read. The invariants below are what the suite needs.
        assert!(
            ds.samples.len() >= 9,
            "the focused dataset lost samples: {}",
            ds.samples.len()
        );

        let mut seen = std::collections::BTreeSet::new();
        for s in &ds.samples {
            assert!(!s.input.is_empty(), "{} has no input", s.id);
            assert!(seen.insert(s.id.clone()), "duplicate sample id {}", s.id);
            // Every sample declares at least one expectation the scorers read.
            let has_expectation = s.metadata.contains_key("expect_tools")
                || s.metadata.contains_key("forbid_tools")
                || s.metadata.contains_key("expect_commands")
                || s.metadata.contains_key("expect_scheduled_agent");
            assert!(has_expectation, "{} declares no scorable expectation", s.id);
            assert!(
                !s.tags.is_empty(),
                "{} has no tags, so no preset selects it",
                s.id
            );
        }
    }

    /// Every `everruns <noun> <verb>` spelling and every `--flag` a case
    /// expects has to exist in the shared command contract.
    ///
    /// A case encoding a flag that does not exist is the same defect as an
    /// example documenting one: it passes review, it fails at runtime, and it
    /// grades the model on something impossible. The contract is generated
    /// from the command types, so checking against it is checking against what
    /// the server will actually accept.
    #[test]
    fn expected_command_lines_exist_in_the_contract() {
        const CONTRACT: &str = include_str!("../../../crates/cli-contract/commands.json");
        let contract: serde_json::Value =
            serde_json::from_str(CONTRACT).expect("commands.json parses");
        let commands = contract.as_array().expect("commands.json is an array");

        let spelling_of = |command: &serde_json::Value| -> String {
            let mut parts: Vec<String> = command["path"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .filter_map(|p| p.as_str().map(ToOwned::to_owned))
                .collect();
            parts.push(command["verb"].as_str().unwrap_or_default().to_string());
            parts.join(" ")
        };

        let ds = Dataset::from_jsonl_str(DATASET).expect("dataset.jsonl parses");
        let spelling =
            regex::Regex::new(r"everruns(?:\\s\+|\s)+([a-z-]+(?:(?:\\s\+|\s)+[a-z-]+)*)")
                .expect("spelling pattern");
        // A flag inside a pattern may carry a character class, because both
        // spellings of a parameter are accepted: `--system[_-]prompt` is one
        // flag, not `--system`. Consume the class, then compare everything in
        // one spelling so the two aliases are the same string here.
        let flag = regex::Regex::new(r"--([a-z][a-z0-9_-]*(?:\[[^\]]*\][a-z0-9_-]*)*)")
            .expect("flag pattern");
        let kebab = |name: &str| {
            let mut out = String::with_capacity(name.len());
            let mut chars = name.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '[' => {
                        for skipped in chars.by_ref() {
                            if skipped == ']' {
                                break;
                            }
                        }
                        out.push('-');
                    }
                    '_' => out.push('-'),
                    other => out.push(other),
                }
            }
            out
        };

        for sample in &ds.samples {
            for key in ["expect_commands", "forbid_commands"] {
                // A forbidden pattern names what the model must not type,
                // which is often a flag that does not exist: that is why it is
                // forbidden. Only its spelling is checkable. An expected
                // pattern names what the model must type, so every flag in it
                // has to be real or the case grades an impossibility.
                let flags_must_exist = key == "expect_commands";
                let Some(entries) = sample.metadata.get(key).and_then(|v| v.as_array()) else {
                    continue;
                };
                for entry in entries {
                    let Some(pattern) = entry.get("regex").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    // Alternation cannot be split on `|`: these patterns
                    // contain `[^\n;|]*`, so splitting tears a flag away from
                    // the command it belongs to and the check silently passes.
                    // Instead, each spelling owns the text up to the next one.
                    let spellings: Vec<_> = spelling.captures_iter(pattern).collect();
                    for (index, found) in spellings.iter().enumerate() {
                        let whole = found.get(0).expect("group 0");
                        let window_end = spellings
                            .get(index + 1)
                            .and_then(|next| next.get(0))
                            .map(|m| m.start())
                            .unwrap_or(pattern.len());
                        let window = &pattern[whole.start()..window_end];

                        let words: Vec<&str> = found[1]
                            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                            .filter(|w| !w.is_empty() && *w != "s")
                            .collect();

                        // The longest prefix of the captured words that names a
                        // real command; a regex tail is not part of it.
                        let matched = (1..=words.len())
                            .rev()
                            .map(|take| words[..take].join(" "))
                            .find(|candidate| {
                                commands.iter().any(|c| &spelling_of(c) == candidate)
                            });
                        let Some(matched) = matched else {
                            panic!(
                                "{}: `everruns {}` is not a command in the contract",
                                sample.id,
                                words.join(" ")
                            );
                        };

                        let command = commands
                            .iter()
                            .find(|c| spelling_of(c) == matched)
                            .expect("just matched");
                        // Both the declared long and the parameter's own name
                        // reach the same argument, so both are legitimate in a
                        // case.
                        let accepted: Vec<String> = command["args"]
                            .as_array()
                            .map(|args| {
                                args.iter()
                                    .flat_map(|a| {
                                        [a["long"].as_str(), a["field"].as_str()]
                                            .into_iter()
                                            .flatten()
                                            .map(kebab)
                                            .collect::<Vec<_>>()
                                    })
                                    .collect::<std::collections::BTreeSet<_>>()
                                    .into_iter()
                                    .collect()
                            })
                            .unwrap_or_default();

                        if flags_must_exist {
                            for found_flag in flag.captures_iter(window) {
                                let name = kebab(&found_flag[1]);
                                assert!(
                                    accepted.contains(&name),
                                    "{}: `everruns {matched}` has no `--{name}`; it accepts {accepted:?}",
                                    sample.id
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn study_builds() {
        // Exercises the registered eval end to end (dataset + subject + scorers).
        let eval = platform_capability();
        assert_eq!(eval.name, "platform_capability");
    }
}
