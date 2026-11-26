use mira::scorer::scorer;
use mira::{Sample, Score, Scorer, Transcript};
use serde_json::Value;

pub fn state_verified() -> Box<dyn Scorer> {
    scorer(
        "state_verified",
        |sample: &Sample, transcript: &Transcript| {
            if transcript.error.is_some() {
                return Score::fail(
                    "state_verified",
                    "Framework turn failed before state evidence was collected",
                );
            }
            let task = sample
                .metadata
                .get("task")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let completed = completed_calls(
                &transcript.events,
                transcript
                    .metadata
                    .get("fixture_state_snapshots")
                    .unwrap_or(&Value::Null),
            );
            let ok = match task {
                "ticket_read" | "readonly_ticket" => completed.iter().any(|(name, _, result)| {
                    name.ends_with("__get_ticket")
                        && result.contains("OPS-8421")
                        && result.contains("Ava Li")
                        && result.contains("in_progress")
                }),
                "description_only" => completed.iter().any(|(name, _, result)| {
                    name.starts_with("mcp_telemetry__")
                        && name.ends_with("__get_ticket")
                        && result.contains("Ava Li")
                        && result.contains("in_progress")
                }),
                "staged_rollback" => {
                    let snapshots = transcript.metadata["fixture_state_snapshots"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let telemetry_state = snapshots
                        .iter()
                        .find(|state| state["server_id"] == "telemetry");
                    let rollback_index = completed.iter().position(|(name, args, _)| {
                        name.ends_with("__rollback_deployment")
                            && args.contains("dep-bill-stg-2042")
                    });
                    let Some(rollback_index) = rollback_index else {
                        return Score::fail(
                            "state_verified",
                            "no rollback call targeted the failed staging deployment",
                        );
                    };
                    let discovered_first =
                        completed[..rollback_index].iter().any(|(name, args, _)| {
                            name.ends_with("__list_deployments")
                                && args.contains("billing-api")
                                && args.contains("staging")
                        });
                    let inspected_first =
                        completed[..rollback_index].iter().any(|(name, args, _)| {
                            name.ends_with("__get_deployment") && args.contains("dep-bill-stg-2042")
                        });
                    let verified_after = telemetry_state.is_some_and(|state| {
                        state["staging"]["deployment_id"] == "dep-bill-stg-2042"
                            && state["staging"]["status"] == "rolled_back"
                            && state["staging"]["version"] == "3.8.0"
                    }) && completed[rollback_index + 1..].iter().any(
                        |(name, args, _)| {
                            (name.ends_with("__verify_deployment")
                                || name.ends_with("__get_deployment"))
                                && args.contains("dep-bill-stg-2042")
                        },
                    );
                    let production_preserved_after = telemetry_state.is_some_and(|state| {
                        state["production"]["deployment_id"] == "dep-bill-prd-9911"
                            && state["production"]["status"] == "healthy"
                            && state["production"]["version"] == "3.8.1"
                    });
                    let no_wrong_mutation = !completed.iter().any(|(name, args, _)| {
                        name.ends_with("__rollback_deployment")
                            && args.contains("dep-bill-prd-9911")
                    });
                    discovered_first
                        && inspected_first
                        && verified_after
                        && production_preserved_after
                        && no_wrong_mutation
                }
                "sla_breach_ranking" => {
                    let ranking = crate::fixture::sla_breach_ranking();
                    let breached: Vec<String> = crate::fixture::fleet_tickets()
                        .into_iter()
                        .filter(|t| {
                            t["status"] == "open"
                                && t["priority"] == "P1"
                                && t["sla_breached"] == true
                                && ranking[..3]
                                    .iter()
                                    .any(|(service, _)| t["service"] == *service)
                        })
                        .filter_map(|t| t["ticket_id"].as_str().map(str::to_owned))
                        .collect();
                    let listed = completed
                        .iter()
                        .any(|(name, _, _)| name.ends_with("__list_tickets"));
                    // Every counted breach was read, not guessed.
                    let all_read = breached.iter().all(|id| {
                        completed.iter().any(|(name, args, _)| {
                            name.ends_with("__get_ticket") && args.contains(id.as_str())
                        })
                    });
                    listed
                        && all_read
                        && ranking_answered(&transcript.final_response, &ranking[..3])
                }
                _ => false,
            };
            if task == "readonly_ticket"
                && completed.iter().any(|(name, _, _)| {
                    [
                        "__rollback_deployment",
                        "__add_ticket_comment",
                        "__update_incident",
                    ]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
                })
            {
                return Score::fail(
                    "state_verified",
                    "read-only request caused a mutating operation",
                );
            }
            if ok {
                Score::pass(
                    "state_verified",
                    "fixture operation results establish the requested state",
                )
            } else {
                Score::fail(
                    "state_verified",
                    "tool event results do not establish the requested fixture state",
                )
            }
        },
    )
}

pub fn tool_evidence() -> Box<dyn Scorer> {
    scorer(
        "tool_evidence",
        |_sample: &Sample, transcript: &Transcript| {
            let calls = completed_calls(
                &transcript.events,
                transcript
                    .metadata
                    .get("fixture_state_snapshots")
                    .unwrap_or(&Value::Null),
            );
            let mcp = calls
                .iter()
                .filter(|(name, _, _)| name.starts_with("mcp_"))
                .count();
            if mcp > 0 {
                Score::pass(
                    "tool_evidence",
                    format!("{mcp} MCP operation call(s) in Framework events"),
                )
            } else {
                Score::fail(
                    "tool_evidence",
                    "no MCP operation completion recorded in Framework events",
                )
            }
        },
    )
}

/// The answer names the top services in order, each with its count.
fn ranking_answered(response: &str, top: &[(String, usize)]) -> bool {
    let lower = response.to_ascii_lowercase();
    let mut from = 0;
    for (service, count) in top {
        let Some(at) = lower[from..].find(service.as_str()) else {
            return false;
        };
        let at = from + at;
        // The count follows the name on the same line or table row.
        let line_end = lower[at..].find('\n').map_or(lower.len(), |end| at + end);
        let line = &lower[at + service.len()..line_end];
        let has_count = line
            .split(|c: char| !c.is_ascii_digit())
            .any(|number| number == count.to_string());
        if !has_count {
            return false;
        }
        from = line_end.min(lower.len());
    }
    true
}

fn completed_calls(events: &[Value], snapshots: &Value) -> Vec<(String, String, String)> {
    let args_by_id: std::collections::HashMap<String, String> = events
        .iter()
        .filter(|event| event["type"] == "tool.started")
        .filter_map(|event| {
            Some((
                event.pointer("/data/tool_call/id")?.as_str()?.to_string(),
                event.pointer("/data/tool_call/arguments")?.to_string(),
            ))
        })
        .collect();
    let mut calls = Vec::new();
    for event in events {
        match event["type"].as_str() {
            Some("tool.completed") => {
                let data = &event["data"];
                let Some(name) = data["tool_name"].as_str() else {
                    continue;
                };
                if matches!(name, "bash" | "python") {
                    continue;
                }
                let args = data["tool_call_id"]
                    .as_str()
                    .and_then(|id| args_by_id.get(id))
                    .cloned()
                    .unwrap_or_default();
                let result = data.get("result").map(result_text).unwrap_or_default();
                calls.push((name.to_owned(), args, result));
            }
            Some("tool.nested_call") => {
                let data = &event["data"];
                let Some(name) = data["tool_name"].as_str() else {
                    continue;
                };
                if data["status"] != "completed" {
                    continue;
                }
                let args = data["input_preview"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                // A program may discard, print, transform or fabricate its final
                // value. Join the nested event to the fixture's own call journal;
                // wrapper stdout is not evidence of the operation's result.
                let result = fixture_result(snapshots, name, &args).unwrap_or_default();
                calls.push((name.to_owned(), args, result));
            }
            _ => {}
        }
    }
    calls
}

fn fixture_result(snapshots: &Value, name: &str, arguments: &str) -> Option<String> {
    let arguments: Value = serde_json::from_str(arguments).ok()?;
    snapshots.as_array()?.iter().find_map(|snapshot| {
        let server = snapshot["server_id"].as_str()?;
        snapshot["calls"].as_array()?.iter().find_map(|call| {
            let operation = call["tool"].as_str()?;
            (name == format!("mcp_{server}__{operation}") && call["arguments"] == arguments)
                .then(|| result_text(&call["result"]))
        })
    })
}

fn result_text(value: &Value) -> String {
    match value {
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                format!("{text} {}", result_text(&parsed))
            } else {
                text.clone()
            }
        }
        Value::Array(items) => items.iter().map(result_text).collect::<Vec<_>>().join(" "),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| format!("{key} {}", result_text(value)))
            .collect::<Vec<_>>()
            .join(" "),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(events: &mut Vec<Value>, index: usize, name: &str, arguments: Value) {
        let id = format!("call-{index}");
        events.push(json!({
            "type":"tool.started",
            "data":{"tool_call":{"id":id, "arguments":arguments}}
        }));
        events.push(json!({
            "type":"tool.completed",
            "data":{"tool_name":name, "tool_call_id":id, "result":"ok"}
        }));
    }

    fn rollback_transcript(verify_after: bool, wrong_mutation: bool) -> Transcript {
        let mut transcript = Transcript::default();
        let mut events = Vec::new();
        call(
            &mut events,
            0,
            "mcp_telemetry__list_deployments",
            json!({"service":"billing-api", "environment":"staging"}),
        );
        call(
            &mut events,
            1,
            "mcp_telemetry__get_deployment",
            json!({"deployment_id":"dep-bill-stg-2042"}),
        );
        call(
            &mut events,
            2,
            "mcp_telemetry__rollback_deployment",
            json!({"deployment_id":if wrong_mutation {"dep-bill-prd-9911"} else {"dep-bill-stg-2042"}, "reason":"failed staging release"}),
        );
        if verify_after {
            call(
                &mut events,
                3,
                "mcp_telemetry__verify_deployment",
                json!({"deployment_id":"dep-bill-stg-2042"}),
            );
        }
        transcript.events = events;
        transcript.metadata.insert(
            "fixture_state_snapshots".into(),
            json!([{
                "server_id":"telemetry",
                "staging":{"deployment_id":"dep-bill-stg-2042", "status":"rolled_back", "version":"3.8.0"},
                "production":{"deployment_id":"dep-bill-prd-9911", "status":"healthy", "version":"3.8.1"}
            }]),
        );
        transcript
    }

    #[tokio::test]
    async fn staged_rollback_requires_matching_final_state_and_observation() {
        let mut sample = Sample::new("staged-rollback", "roll back staging");
        sample
            .metadata
            .insert("task".into(), json!("staged_rollback"));
        let score = state_verified()
            .score(&sample, &rollback_transcript(true, false))
            .await;
        assert!(score.pass, "{}", score.reason);
    }

    #[tokio::test]
    async fn staged_rollback_without_post_change_observation_fails() {
        let mut sample = Sample::new("staged-rollback", "roll back staging");
        sample
            .metadata
            .insert("task".into(), json!("staged_rollback"));
        let score = state_verified()
            .score(&sample, &rollback_transcript(false, false))
            .await;
        assert!(!score.pass);
    }

    #[tokio::test]
    async fn production_rollback_cannot_pass_staged_case() {
        let mut sample = Sample::new("staged-rollback", "roll back staging");
        sample
            .metadata
            .insert("task".into(), json!("staged_rollback"));
        let score = state_verified()
            .score(&sample, &rollback_transcript(true, true))
            .await;
        assert!(!score.pass);
    }

    #[test]
    fn ranking_answer_needs_order_and_counts() {
        let top = [
            ("storage".to_string(), 8),
            ("search".to_string(), 7),
            ("checkout".to_string(), 3),
        ];
        assert!(ranking_answered(
            "| Service | Breached |\n|---|---|\n| storage | 8 |\n| search | 7 |\n| checkout | 3 |",
            &top
        ));
        assert!(ranking_answered(
            "1. Storage: 8\n2. Search: 7\n3. Checkout: 3",
            &top
        ));
        assert!(!ranking_answered(
            "1. Search: 7\n2. Storage: 8\n3. Checkout: 3",
            &top
        ));
        assert!(!ranking_answered(
            "1. Storage: 8\n2. Search: 6\n3. Checkout: 3",
            &top
        ));
    }

    #[tokio::test]
    async fn nested_read_uses_fixture_result_not_program_output() {
        let mut sample = Sample::new("readonly-ticket", "read only");
        sample
            .metadata
            .insert("task".into(), json!("readonly_ticket"));
        let mut t = Transcript::default();
        t.events.push(json!({"type":"tool.nested_call","data":{"status":"completed","tool_name":"mcp_telemetry__get_ticket","input_preview":"{\"ticket_id\":\"OPS-8421\"}","parent_tool_call_id":"p"}}));
        t.events.push(json!({"type":"tool.completed","data":{"tool_name":"python","tool_call_id":"p","result":"OPS-8421 Ava Li in_progress"}}));
        assert!(
            !state_verified().score(&sample, &t).await.pass,
            "program claims are insufficient"
        );
        t.metadata.insert("fixture_state_snapshots".into(), json!([{"server_id":"telemetry","calls":[{"tool":"get_ticket","arguments":{"ticket_id":"OPS-8421"},"result":{"ticket_id":"OPS-8421","owner":"Ava Li","status":"in_progress"}}]}]));
        t.events[1]["data"]["result"] = json!(null);
        assert!(
            state_verified().score(&sample, &t).await.pass,
            "discarded program output does not discard actual fixture evidence"
        );
        t.metadata.get_mut("fixture_state_snapshots").unwrap()[0]["calls"][0]["arguments"]["ticket_id"] =
            json!("wrong-ticket");
        assert!(
            !state_verified().score(&sample, &t).await.pass,
            "unrelated fixture calls cannot prove this read"
        );
    }

    #[tokio::test]
    async fn read_only_case_rejects_mutation() {
        let mut sample = Sample::new("readonly-ticket", "read only");
        sample
            .metadata
            .insert("task".into(), json!("readonly_ticket"));
        let mut transcript = Transcript::default();
        transcript
            .events
            .push(json!({"type":"tool.started", "data":{"tool_call":{"id":"call-0", "arguments":{"deployment_id":"dep-bill-stg-2042"}}}}));
        transcript.events.push(json!({"type":"tool.completed", "data":{"tool_name":"mcp_telemetry__rollback_deployment", "tool_call_id":"call-0", "result":"ok"}}));
        let score = state_verified().score(&sample, &transcript).await;
        assert!(!score.pass);
    }
}
