//! Mira eval study: **prompt caching as a session grows**.
//!
//! Each case plays a multi-turn conversation against a running Everruns
//! server and reads every model call's cache usage back from the session's
//! `llm.generation` events. A healthy session only appends to its previous
//! request, so from the second call on most of each prompt is a cache read and
//! the share should hold, or rise, as the history grows. When a call loses the
//! cache, the study diffs the two stored request bodies and names what changed
//! first: model, cache key, instructions, tools or an earlier message
//! (`cache.rs`).
//!
//! ```bash
//! EVERRUNS_API_URL=http://localhost:27100/api \
//!   ./target/debug/prompt_cache_growth --run
//! mira --bin prompt_cache_growth run --preset smoke
//! ```

#[path = "../../common/everruns_usage.rs"]
mod everruns_usage;

mod cache;
mod subject;

use mira::scorer::{scorer, succeeded};
use mira::{Dataset, Eval, Sample, Score, Scorer, Target, Transcript, Trial, eval};
use serde_json::Value;

use crate::subject::{BREAKS_KEY, ServerSubject};

const DATASET: &str = include_str!("../dataset.jsonl");

/// A session should read at least this share of its prompts from cache when a
/// case does not set `min_hit_rate`.
const DEFAULT_MIN_HIT_RATE: f64 = 0.6;

/// Targets are models on the server: a `model_…` id, a `model_id` or a
/// display name (`EVERRUNS_EVAL_TARGETS`, comma-separated). The default pair
/// covers both tool-change paths: Muse through OpenRouter receives the tool
/// list, GPT-6 Luna receives tool changes inline.
fn targets() -> Vec<Target> {
    let spec = std::env::var("EVERRUNS_EVAL_TARGETS").unwrap_or_default();
    let spec = if spec.trim().is_empty() {
        "meta/muse-spark-1.3-contributor,gpt-6-luna".to_string()
    } else {
        spec
    };
    spec.split(',')
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|model| Target::new(model, "everruns", model))
        .collect()
}

/// `cache_hit_rate`: share of prompt tokens read from cache after the first
/// call, against the case's `min_hit_rate`.
fn cache_hit_rate() -> Box<dyn Scorer> {
    scorer("cache_hit_rate", |sample: &Sample, t: &Transcript| {
        let Some(rate) = t.metric("cache.hit_rate") else {
            return Score::na("cache_hit_rate", "fewer than two model calls with usage");
        };
        let min = sample
            .metadata
            .get("min_hit_rate")
            .and_then(Value::as_f64)
            .unwrap_or(DEFAULT_MIN_HIT_RATE);
        let late = t
            .metric("cache.hit_rate_late")
            .map(|late| format!(", {:.0}% in the later half", late * 100.0))
            .unwrap_or_default();
        Score::graded(
            "cache_hit_rate",
            rate,
            min,
            format!(
                "{:.0}% of prompt tokens cached{late} (need {:.0}%)",
                rate * 100.0,
                min * 100.0
            ),
        )
    })
}

/// `prefix_stable`: no call lost the cache to a change Everruns controls (cache
/// key, instructions, a reordered tool list, a rewritten earlier message).
fn prefix_stable() -> Box<dyn Scorer> {
    scorer("prefix_stable", |_: &Sample, t: &Transcript| {
        if t.metric("cache.breaks").is_none() {
            return Score::na("prefix_stable", "no cache accounting");
        }
        let defects: Vec<String> = cache::Cause::ALL
            .into_iter()
            .filter(|cause| cause.is_prefix_defect())
            .filter(|cause| {
                t.metric(&format!("cache.breaks.{}", cause.as_str()))
                    .unwrap_or(0.0)
                    > 0.0
            })
            .map(|cause| cause.as_str().to_string())
            .collect();
        if defects.is_empty() {
            return Score::pass("prefix_stable", "every request extended the previous one");
        }
        let detail = t
            .metadata
            .get(BREAKS_KEY)
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        Score::fail(
            "prefix_stable",
            format!("prefix rewritten ({}): {detail}", defects.join(", ")),
        )
    })
}

#[eval]
fn prompt_cache_growth() -> Eval {
    let dataset = Dataset::from_jsonl_str(DATASET).expect("embedded dataset.jsonl must parse");
    Eval::new("prompt_cache_growth")
        .describe("Prompt cache reuse across a growing hosted session")
        .dataset(dataset)
        .targets(targets())
        .subject(ServerSubject::from_env())
        .scorer(succeeded())
        .scorer(cache_hit_rate())
        .scorer(prefix_stable())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--run") {
        return mira::Study::registered().serve().await;
    }
    let filter = args
        .iter()
        .position(|a| a == "--filter")
        .and_then(|i| args.get(i + 1));

    // Sequential on purpose: concurrent sessions share the provider's cache
    // capacity and would blur each other's hit rates.
    let mut failed = 0;
    for eval in mira::registered_evals() {
        for target in &eval.targets {
            for sample in &eval.dataset.samples {
                let key =
                    mira::case_key(&eval.name, &sample.id, &target.label, &Default::default());
                if filter.is_some_and(|f| !key.contains(f.as_str())) {
                    continue;
                }
                let outcome = mira::runner::run_case(
                    &eval,
                    sample,
                    target,
                    &Default::default(),
                    Trial::default(),
                )
                .await;
                failed += usize::from(!outcome.passed);
                let t = &outcome.transcript;
                let pct = |name: &str| {
                    t.metric(name)
                        .map(|v| format!("{:>3.0}%", v * 100.0))
                        .unwrap_or_else(|| "  - ".to_string())
                };
                println!(
                    "{}  {key}\n      calls {:>3}  cached {}  late {}  breaks {:>2}  cost ${:.4}",
                    if outcome.passed { "PASS" } else { "FAIL" },
                    t.metric("llm.calls").unwrap_or(0.0),
                    pct("cache.hit_rate"),
                    pct("cache.hit_rate_late"),
                    t.metric("cache.breaks").unwrap_or(0.0),
                    t.metric("llm.provider_reported_cost_usd").unwrap_or(0.0),
                );
                for line in t
                    .metadata
                    .get(BREAKS_KEY)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    println!("        break {line}");
                }
                for score in outcome.scores.iter().filter(|s| !s.pass && !s.na) {
                    println!("        {}: {}", score.scorer, score.reason);
                }
                if let Some(error) = &t.error {
                    println!("        error: {error}");
                }
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dataset_cases_grow_and_have_unique_ids() {
        let dataset = Dataset::from_jsonl_str(DATASET).expect("dataset.jsonl parses");
        let mut seen = std::collections::HashSet::new();
        for sample in &dataset.samples {
            assert!(seen.insert(sample.id.clone()), "duplicate id {}", sample.id);
            assert!(
                sample.input.len() >= 5,
                "{} needs enough turns to grow",
                sample.id
            );
            assert!(!sample.tags.is_empty(), "{} has no tags", sample.id);
        }
        assert!(
            dataset
                .samples
                .iter()
                .any(|s| s.tags.iter().any(|t| t == "smoke"))
        );
    }

    fn transcript_from(events: &[Value]) -> Transcript {
        let mut t = Transcript::default();
        subject::record(&mut t, &cache::Report::from_events(events));
        t
    }

    fn generation(sequence: i64, input: u64, cached: u64, history: &[&str]) -> Value {
        json!({
            "type": "llm.generation",
            "sequence": sequence,
            "data": {
                "metadata": { "usage": { "input_tokens": input, "cache_read_tokens": cached } },
                "request_body": { "model": "m", "input": history },
            },
        })
    }

    #[tokio::test]
    async fn scorers_fail_a_rewritten_history_and_pass_an_appending_one() {
        let sample = Sample::new("s", "hi");
        let good = transcript_from(&[
            generation(1, 4000, 0, &["u1"]),
            generation(2, 100, 4000, &["u1", "r1", "u2"]),
        ]);
        assert!(cache_hit_rate().score(&sample, &good).await.pass);
        assert!(prefix_stable().score(&sample, &good).await.pass);

        let bad = transcript_from(&[
            generation(1, 4000, 0, &["u1", "t1", "u2"]),
            generation(2, 4100, 0, &["u1", "t1 masked", "u2", "u3"]),
        ]);
        assert!(!cache_hit_rate().score(&sample, &bad).await.pass);
        let stable = prefix_stable().score(&sample, &bad).await;
        assert!(!stable.pass);
        assert!(
            stable.reason.contains("history_rewritten at history[1]"),
            "{}",
            stable.reason
        );
    }
}
