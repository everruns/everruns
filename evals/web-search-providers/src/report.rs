//! The sweep without the Mira host: every provider over the whole corpus,
//! written as `cases.jsonl` (one line per provider and case) and `summary.md`.

use std::collections::BTreeMap;
use std::path::Path;

use futures::StreamExt;
use mira::{Sample, Transcript};
use serde_json::{Value, json};

use crate::subject::{ERROR_KEY, SKIPPED_KEY, fresh, run_case};

/// Concurrent queries per provider. Free search paces itself; Brave's plans
/// allow one request a second.
fn concurrency(provider: &str) -> usize {
    match provider {
        "free" | "brave" => 1,
        // The three Cloudflare indexes share one account's rate limit.
        "ceramic" | "exa" | "linkup" => 2,
        _ => 4,
    }
}

pub async fn run(out: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(out)?;
    let samples = crate::dataset().samples;
    let providers = crate::provider_axis();
    let runs = providers.iter().map(|provider| {
        let samples = samples.clone();
        async move {
            let results: Vec<(Sample, Transcript)> = futures::stream::iter(samples)
                .map(|sample| async move {
                    let t = run_case(provider, &sample).await;
                    eprintln!("{provider:<9} {:<26} {}", sample.id, brief(&t));
                    (sample, t)
                })
                .buffer_unordered(concurrency(provider))
                .collect()
                .await;
            (provider.clone(), results)
        }
    });
    let all: Vec<(String, Vec<(Sample, Transcript)>)> = futures::future::join_all(runs).await;

    let mut lines = String::new();
    for (provider, results) in &all {
        for (sample, t) in results {
            let row = json!({
                "provider": provider,
                "case": sample.id,
                "category": sample.tags.first(),
                "query": sample.input.join(" "),
                "fresh": fresh(sample),
                "metadata": t.metadata,
            });
            lines.push_str(&row.to_string());
            lines.push('\n');
        }
    }
    std::fs::write(out.join("cases.jsonl"), lines)?;
    let summary = summarize(&all);
    std::fs::write(out.join("summary.md"), &summary)?;
    println!("{summary}");
    Ok(())
}

fn brief(t: &Transcript) -> String {
    let m = &t.metadata;
    if let Some(reason) = m.get(SKIPPED_KEY) {
        return format!("skipped: {reason}");
    }
    if let Some(error) = m.get(ERROR_KEY) {
        return format!("error: {error}");
    }
    format!(
        "answerable={} ndcg={:.2} {}ms",
        m.get("answerable").unwrap_or(&Value::Null),
        m.get("ndcg").and_then(Value::as_f64).unwrap_or(0.0),
        m.get("latency_ms").unwrap_or(&Value::Null),
    )
}

#[derive(Default)]
struct Tally {
    cases: usize,
    answerable: usize,
    ndcg: f64,
    fresh: usize,
    current: usize,
    failed: usize,
    empty: usize,
    latencies: Vec<u64>,
    cost: f64,
    priced: usize,
    rate_limited: u64,
}

fn summarize(all: &[(String, Vec<(Sample, Transcript)>)]) -> String {
    let mut md = String::from(
        "| Provider | Answerable | nDCG@5 | Current (time-sensitive) | Failed | Empty | Median latency | Cost per 1,000 | 429s retried |\n\
         |---|---|---|---|---|---|---|---|---|\n",
    );
    let mut by_category: BTreeMap<String, BTreeMap<String, (usize, usize)>> = BTreeMap::new();
    for (provider, results) in all {
        let mut tally = Tally::default();
        for (sample, t) in results {
            let m = &t.metadata;
            if m.contains_key(SKIPPED_KEY) {
                continue;
            }
            tally.cases += 1;
            let ok = m.get("answerable").and_then(Value::as_bool) == Some(true);
            tally.answerable += usize::from(ok);
            tally.ndcg += m.get("ndcg").and_then(Value::as_f64).unwrap_or(0.0);
            if fresh(sample) {
                tally.fresh += 1;
                tally.current +=
                    usize::from(m.get("current").and_then(Value::as_bool) == Some(true));
            }
            if m.contains_key(ERROR_KEY) {
                tally.failed += 1;
            } else if m
                .get("hits")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
            {
                tally.empty += 1;
            }
            tally.rate_limited += m.get("rate_limited").and_then(Value::as_u64).unwrap_or(0);
            if let Some(ms) = m.get("latency_ms").and_then(Value::as_u64) {
                tally.latencies.push(ms);
            }
            if let Some(cost) = m.get("cost_usd").and_then(Value::as_f64) {
                tally.cost += cost;
                tally.priced += 1;
            }
            let category = sample.tags.first().cloned().unwrap_or_default();
            let cell = by_category
                .entry(category)
                .or_default()
                .entry(provider.clone())
                .or_default();
            cell.0 += usize::from(ok);
            cell.1 += 1;
        }
        if tally.cases == 0 {
            md.push_str(&format!("| {provider} | skipped | | | | | | | |\n"));
            continue;
        }
        tally.latencies.sort_unstable();
        let median = tally
            .latencies
            .get(tally.latencies.len() / 2)
            .copied()
            .unwrap_or(0);
        let per_k = if tally.priced == 0 {
            0.0
        } else {
            tally.cost / tally.priced as f64 * 1000.0
        };
        md.push_str(&format!(
            "| {provider} | {}/{} ({:.0}%) | {:.2} | {}/{} | {} | {} | {:.1} s | ${:.2} | {} |\n",
            tally.answerable,
            tally.cases,
            100.0 * tally.answerable as f64 / tally.cases as f64,
            tally.ndcg / tally.cases as f64,
            tally.current,
            tally.fresh,
            tally.failed,
            tally.empty,
            median as f64 / 1000.0,
            per_k,
            tally.rate_limited,
        ));
    }

    let providers: Vec<&String> = all.iter().map(|(p, _)| p).collect();
    md.push_str("\nAnswerable by category:\n\n| Category |");
    for p in &providers {
        md.push_str(&format!(" {p} |"));
    }
    md.push_str("\n|---|");
    md.push_str(&"---|".repeat(providers.len()));
    md.push('\n');
    for (category, cells) in &by_category {
        md.push_str(&format!("| {category} |"));
        for p in &providers {
            match cells.get(*p) {
                Some((ok, n)) => md.push_str(&format!(" {ok}/{n} |")),
                None => md.push_str(" |"),
            }
        }
        md.push('\n');
    }
    md
}
