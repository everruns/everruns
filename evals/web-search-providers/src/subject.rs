//! Runs one query through one provider and has the judge grade the list.
//!
//! The provider is the `provider` axis. A provider without credentials skips
//! rather than scoring: an unconfigured provider is unmeasured, not bad.

use mira::{RunCx, Sample, Subject, Transcript};
use serde_json::json;

use crate::judge;
use crate::providers;

pub const SKIPPED_KEY: &str = "skipped";
pub const ERROR_KEY: &str = "search_error";
pub const DEFAULT_PROVIDER: &str = "ceramic";

pub fn query(sample: &Sample) -> String {
    sample.input.join(" ")
}

pub fn need(sample: &Sample) -> &str {
    sample
        .metadata
        .get("need")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
}

pub fn fresh(sample: &Sample) -> bool {
    sample
        .metadata
        .get("fresh")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub struct SearchProviderSubject;

#[async_trait::async_trait]
impl Subject for SearchProviderSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let provider = cx.param("provider").unwrap_or(DEFAULT_PROVIDER).to_string();
        run_case(&provider, sample).await
    }
}

pub async fn run_case(provider: &str, sample: &Sample) -> Transcript {
    let mut t = Transcript::default();
    t.metadata.insert("provider".into(), provider.into());
    if let Some(missing) = providers::missing_credential(provider) {
        t.metadata.insert(
            SKIPPED_KEY.into(),
            format!("{provider}: {missing} is not set").into(),
        );
        return t;
    }
    let query = query(sample);
    let outcome = match providers::search(provider, &query).await {
        Ok(outcome) => outcome,
        Err(error) => {
            // A failed search is a result: Automatic would fall back, and the
            // failure rate is part of what this study reports.
            t.metadata.insert(ERROR_KEY.into(), error.into());
            t.metadata.insert("answerable".into(), false.into());
            t.metadata.insert("ndcg".into(), 0.0.into());
            if fresh(sample) {
                t.metadata.insert("current".into(), false.into());
            }
            return t;
        }
    };
    let cost = providers::price_per_search(provider) + outcome.extra_cost_usd;
    t.metadata
        .insert("latency_ms".into(), outcome.latency_ms.into());
    t.metadata
        .insert("rate_limited".into(), outcome.rate_limited.into());
    t.metadata.insert("cost_usd".into(), cost.into());
    t.metadata.insert(
        "hits".into(),
        json!(
            outcome
                .hits
                .iter()
                .map(|h| json!({"title": h.title, "url": h.url, "published": h.published}))
                .collect::<Vec<_>>()
        ),
    );
    t.final_response = outcome
        .hits
        .iter()
        .map(|h| format!("{} <{}>", h.title, h.url))
        .collect::<Vec<_>>()
        .join("\n");
    t = t.with_duration_ms(outcome.latency_ms);
    match judge::grade(&query, need(sample), fresh(sample), &outcome.hits).await {
        Ok(verdict) => {
            let ndcg = judge::ndcg(&verdict.grades, providers::LIMIT as usize);
            t.metadata.insert("grades".into(), json!(verdict.grades));
            t.metadata.insert("ndcg".into(), ndcg.into());
            t.metadata
                .insert("answerable".into(), verdict.answerable.into());
            if let Some(current) = verdict.current {
                t.metadata.insert("current".into(), current.into());
            }
            t.metadata.insert("judge_note".into(), verdict.note.into());
        }
        Err(error) => {
            t.metadata
                .insert(SKIPPED_KEY.into(), format!("judge failed: {error}").into());
        }
    }
    t
}
