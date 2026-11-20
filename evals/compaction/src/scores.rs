use mira::{Sample, Score, Scorer, Transcript, scorer::scorer};
use serde_json::Value;

fn parse(response: &str) -> Option<Value> {
    let response = response.trim();
    let response = if response.starts_with("```") {
        response
            .lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end_matches('`')
            .trim()
            .to_owned()
    } else {
        response.to_owned()
    };
    serde_json::from_str(&response).ok()
}

fn check(t: &Transcript, citations: bool) -> Vec<String> {
    let mut failures = vec![];
    if let Some(e) = &t.error {
        failures.push(format!("execution: {e}"));
    }
    let Some(phases) = t.metadata.get("phases").and_then(Value::as_array) else {
        return vec!["no phases recorded".into()];
    };
    if phases.len() != 5 {
        failures.push(format!("only {} of 5 phases completed", phases.len()));
    }
    for phase in phases {
        let name = phase["phase"].as_str().unwrap_or("?");
        let Some(answer) = phase["response"].as_str().and_then(parse) else {
            failures.push(format!("{name}: invalid JSON"));
            continue;
        };
        let Some(expected) = phase["expected"].as_object() else {
            failures.push(format!("{name}: no oracle"));
            continue;
        };
        for (key, fact) in expected {
            // A missing wrapper is an output-contract failure, not proof that
            // the underlying fact or its provenance disappeared.
            let actual = &answer.get("facts").unwrap_or(&answer)[key];
            if citations {
                let source_ids = actual["source_ids"].as_array().cloned().or_else(|| {
                    actual
                        .get("source_message_id")
                        .filter(|id| id.is_string())
                        .map(|id| vec![id.clone()])
                });
                if !source_ids.is_some_and(|ids| {
                    ids.contains(&fact["source_id"])
                        || fact["equivalent_source_ids"]
                            .as_array()
                            .is_some_and(|valid| valid.iter().any(|id| ids.contains(id)))
                }) {
                    failures.push(format!(
                        "{name}/{key}: missing exact authoritative source {}",
                        fact["source_id"]
                    ));
                }
            } else if actual["value"] != fact["value"]
                && !fact["accepted_values"]
                    .as_array()
                    .is_some_and(|values| values.contains(&actual["value"]))
            {
                failures.push(format!(
                    "{name}/{key}: expected {}, got {}",
                    fact["value"], actual["value"]
                ));
            }
        }
    }
    failures
}

fn score(name: &str, failures: Vec<String>) -> Score {
    if failures.is_empty() {
        Score::pass(name, "all phases passed")
    } else {
        Score::fail(name, failures.join("; "))
    }
}

pub fn fidelity() -> Box<dyn Scorer> {
    scorer("fidelity", |_: &Sample, t: &Transcript| {
        score("fidelity", check(t, false))
    })
}
pub fn citations() -> Box<dyn Scorer> {
    scorer("source_citations", |_: &Sample, t: &Transcript| {
        score("source_citations", check(t, true))
    })
}
pub fn output_schema() -> Box<dyn Scorer> {
    scorer("output_schema", |_: &Sample, t: &Transcript| {
        let mut failures = vec![];
        let phases = t.metadata.get("phases").and_then(Value::as_array);
        if phases.is_none_or(|p| p.len() != 5) {
            failures.push("five complete probe responses required".into());
        }
        for phase in phases.into_iter().flatten() {
            let answer = phase["response"]
                .as_str()
                .and_then(parse)
                .unwrap_or(Value::Null);
            let valid = phase["expected"].as_object().is_some_and(|expected| {
                expected.keys().all(|key| {
                    let fact = &answer["facts"][key];
                    fact["value"].is_string()
                        && fact["source_ids"]
                            .as_array()
                            .is_some_and(|ids| !ids.is_empty() && ids.iter().all(Value::is_string))
                })
            });
            if !valid {
                failures.push(format!(
                    "{}: required facts wrapper/string values/source ID array missing",
                    phase["phase"]
                ));
            }
        }
        score("output_schema", failures)
    })
}
pub fn mechanics() -> Box<dyn Scorer> {
    scorer("mechanics", |_: &Sample, t: &Transcript| {
        let mut failures = vec![];
        if t.metadata.get("mechanics").is_none_or(|m| {
            m["raw_journal_unchanged"] != true || m["portable_after_switch"] != true
        }) {
            failures.push("raw journal or provider-switch continuity failed".into());
        }
        let checkpoints = t.metadata.get("checkpoints").and_then(Value::as_array);
        if !checkpoints.is_some_and(|cp| {
            cp.len() == 3
                && cp.iter().all(|x| x.is_object())
                && cp
                    .windows(2)
                    .all(|p| p[1]["source_sequence"].as_i64() > p[0]["source_sequence"].as_i64())
        }) {
            failures.push("three advancing durable boundaries were not observed".into());
        }
        if t.tool_calls.iter().any(|name| name != "query_history") {
            failures.push("unexpected tool attempted".into());
        }
        score("mechanics", failures)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn correct_words_with_wrong_provenance_do_not_pass() {
        let mut t = Transcript::default();
        t.metadata.insert("phases".into(),json!((0..5).map(|_|json!({"phase":"test","expected":{"approval":{"value":"revoked","source_id":"message_actual"}},"response":"{\"facts\":{\"approval\":{\"value\":\"revoked\",\"source_ids\":[\"message_fabricated\"]}}}"})).collect::<Vec<_>>()));
        assert!(check(&t, false).is_empty());
        assert_eq!(check(&t, true).len(), 5);
    }
    #[test]
    fn partial_execution_and_missing_answer_fail() {
        let mut t = Transcript::default();
        t.metadata.insert("phases".into(), json!([]));
        assert!(!check(&t, false).is_empty());
        assert!(parse("not JSON").is_none());
        assert!(parse("```json\n{\"facts\":{}}\n```").is_some());
    }

    #[tokio::test]
    async fn format_failure_stays_visible_without_becoming_memory_loss() {
        for response in [
            "{\"approval\":{\"value\":\"revoked\",\"source_ids\":[\"message_actual\"]}}",
            "{\"approval\":{\"value\":\"revoked\",\"source_message_id\":\"message_actual\"}}",
        ] {
            let mut t = Transcript::default();
            t.metadata.insert("phases".into(),json!((0..5).map(|_|json!({"phase":"test","expected":{"approval":{"value":"revoked","source_id":"message_actual"}},"response":response})).collect::<Vec<_>>()));
            assert!(check(&t, false).is_empty());
            assert!(check(&t, true).is_empty());
            assert!(
                !output_schema()
                    .score(&Sample::new("test", "test"), &t)
                    .await
                    .pass
            );
        }
    }
}
