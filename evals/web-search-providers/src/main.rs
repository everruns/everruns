//! Mira study: **result quality of the web search providers**.
//!
//! Which search should a hosted agent get by default? The providers differ by
//! two orders of magnitude in price (Ceramic $0.25 per 1,000, native about
//! $10), and the only thing price does not tell is whether the cheap list is
//! good enough. This study runs real agent-style queries through each
//! provider's shipped driver and has an LLM judge grade the result lists.
//!
//! ```sh
//! doppler run -- mira --bin web_search_providers
//! # or without the Mira host, writing cases and a summary table:
//! doppler run -- cargo run --release -- report results/
//! ```
//!
//! `WEB_SEARCH_PROVIDERS` narrows the provider axis (default: all of
//! `free,brave,parallel,ceramic,exa,linkup,native`). Providers without
//! credentials skip.

mod judge;
mod providers;
mod report;
mod scorers;
mod subject;

use mira::{Dataset, Eval, Target, eval};

use crate::scorers::{answerable, current, relevance, returned};
use crate::subject::SearchProviderSubject;

/// The dataset travels with the binary so the study runs from any directory.
const DATASET: &str = include_str!("../dataset.jsonl");

pub fn dataset() -> Dataset {
    Dataset::from_jsonl_str(DATASET).expect("embedded dataset.jsonl must parse")
}

pub fn provider_axis() -> Vec<String> {
    let raw = std::env::var("WEB_SEARCH_PROVIDERS").unwrap_or_default();
    let chosen: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect();
    if chosen.is_empty() {
        providers::ALL.iter().map(|p| p.to_string()).collect()
    } else {
        chosen
    }
}

#[eval]
fn web_search_providers() -> Eval {
    Eval::new("web-search-providers")
        .describe("Result quality, freshness, failures and cost of each web search provider")
        .dataset(dataset())
        // The search provider is the subject; the judge model is fixed.
        .targets(vec![Target::new("judge", "openai", judge::JUDGE_MODEL)])
        .axis("provider", provider_axis())
        .subject(SearchProviderSubject)
        .scorer(answerable())
        .scorer(relevance())
        .scorer(current())
        .scorer(returned())
        .build()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("report") {
        let out = args.get(1).cloned().unwrap_or_else(|| "results".into());
        return report::run(std::path::Path::new(&out)).await;
    }
    mira::Study::registered().serve().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_states_its_need() {
        for sample in &dataset().samples {
            assert!(
                !subject::need(sample).is_empty(),
                "{}: the judge needs the need",
                sample.id
            );
            assert_eq!(sample.tags.len(), 1, "{}: one category tag", sample.id);
        }
    }

    #[test]
    fn the_corpus_covers_fresh_and_stable_queries() {
        let samples = dataset().samples;
        let fresh = samples.iter().filter(|s| subject::fresh(s)).count();
        assert!(fresh >= 10, "only {fresh} time-sensitive cases");
        assert!(samples.len() - fresh >= 30);
    }

    #[test]
    fn every_provider_on_the_axis_is_known() {
        for provider in providers::ALL {
            assert_ne!(
                providers::missing_credential(provider),
                Some("unknown provider")
            );
        }
    }
}
