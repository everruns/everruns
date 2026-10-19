//! Grades one provider's result list for one query with an LLM judge.
//!
//! Decision: the judge sees what an agent sees (title, URL, date, snippet) and
//! not the pages themselves. Search quality for an agent is whether the list
//! points it at pages worth fetching; fetching every hit would grade fetchkit
//! as much as the search index.
//!
//! The judge is `gpt-6.1-sol`, a different model from native search's
//! `gpt-6-luna`, and it never sees which provider produced the list.

use everruns_contracts::runtime::web_search::SearchHit;
use serde::Deserialize;
use serde_json::{Value, json};

pub const JUDGE_MODEL: &str = "gpt-6.1-sol";

/// What the judge returns for one result list.
#[derive(Debug, Clone, Deserialize)]
pub struct Verdict {
    /// One grade per hit, 0 to 3.
    pub grades: Vec<u8>,
    /// Whether an agent could meet the need from these results.
    pub answerable: bool,
    /// For time-sensitive queries: whether the results reflect the current
    /// state rather than an outdated one. `None` otherwise.
    #[serde(default)]
    pub current: Option<bool>,
    #[serde(default)]
    pub note: String,
}

const RUBRIC: &str = "\
You grade web search results for an AI agent that will read them and may fetch the pages.
Today is 2026-10-12.

Grade every result 0 to 3:
3 = directly meets the need, from an authoritative or primary source
2 = relevant and useful toward the need
1 = on the topic but not useful for the need (thin, generic, wrong version)
0 = irrelevant, spam, or broken

answerable: true when an agent could meet the need from these results (reading the snippets
or fetching the listed pages), false otherwise.

current: only when the query is time-sensitive. true when the results show the state as of
today (judge from dates, version numbers and years visible in titles, URLs and snippets);
false when they mostly describe an outdated state or nothing shows how recent they are.
null when the query is not time-sensitive.

Reply with JSON only: {\"grades\": [..], \"answerable\": bool, \"current\": bool|null, \"note\": \"one short sentence\"}";

pub async fn grade(
    query: &str,
    need: &str,
    fresh: bool,
    hits: &[SearchHit],
) -> Result<Verdict, String> {
    if hits.is_empty() {
        return Ok(Verdict {
            grades: Vec::new(),
            answerable: false,
            current: fresh.then_some(false),
            note: "no results".into(),
        });
    }
    let key = std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY is not set")?;
    let listing: Vec<Value> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| {
            json!({
                "n": i + 1,
                "title": hit.title,
                "url": hit.url,
                "published": hit.published,
                "snippet": hit.snippet.as_deref().or(hit.excerpt.as_deref())
                    .map(|s| s.chars().take(400).collect::<String>()),
            })
        })
        .collect();
    let input = json!({
        "query": query,
        "need": need,
        "time_sensitive": fresh,
        "results": listing,
    });
    let body = json!({
        "model": JUDGE_MODEL,
        "reasoning": { "effort": "low" },
        "instructions": RUBRIC,
        "input": format!("Grade these results and reply in JSON.\n{input}"),
        "text": { "format": { "type": "json_object" } },
    });
    let client = reqwest::Client::new();
    let mut last_error = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2 << attempt)).await;
        }
        let response = client
            .post("https://api.openai.com/v1/responses")
            .bearer_auth(&key)
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await;
        let value: Value = match response {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    last_error = e.to_string();
                    continue;
                }
            },
            Err(e) => {
                last_error = e.to_string();
                continue;
            }
        };
        if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
            last_error = error.to_string();
            continue;
        }
        let text = output_text(&value);
        match serde_json::from_str::<Verdict>(&text) {
            Ok(mut verdict) => {
                verdict.grades.truncate(hits.len());
                verdict.grades.iter_mut().for_each(|g| *g = (*g).min(3));
                if !fresh {
                    verdict.current = None;
                }
                return Ok(verdict);
            }
            Err(e) => last_error = format!("unparseable verdict ({e}): {text}"),
        }
    }
    Err(last_error)
}

fn output_text(response: &Value) -> String {
    response["output"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "output_text")
        .filter_map(|part| part["text"].as_str())
        .collect()
}

/// Normalized discounted gain of the grades, 0 to 1: a perfect list is a 3
/// in every slot the provider was asked for.
pub fn ndcg(grades: &[u8], slots: usize) -> f64 {
    let gain = |g: f64, i: usize| g / ((i + 2) as f64).log2();
    let actual: f64 = grades
        .iter()
        .take(slots)
        .enumerate()
        .map(|(i, g)| gain(f64::from(*g), i))
        .sum();
    let ideal: f64 = (0..slots).map(|i| gain(3.0, i)).sum();
    if ideal == 0.0 { 0.0 } else { actual / ideal }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndcg_rewards_good_results_early() {
        assert!((ndcg(&[3, 3, 3, 3, 3], 5) - 1.0).abs() < 1e-9);
        assert_eq!(ndcg(&[], 5), 0.0);
        assert!(ndcg(&[3, 0, 0], 5) > ndcg(&[0, 0, 3], 5));
        // Fewer results than slots costs the missing slots.
        assert!(ndcg(&[3], 5) < ndcg(&[3, 3], 5));
    }

    #[test]
    fn verdict_parses_the_rubric_shape() {
        let v: Verdict = serde_json::from_str(
            r#"{"grades":[3,1,0],"answerable":true,"current":null,"note":"ok"}"#,
        )
        .unwrap();
        assert_eq!(v.grades, vec![3, 1, 0]);
        assert!(v.answerable);
        assert_eq!(v.current, None);
    }
}
