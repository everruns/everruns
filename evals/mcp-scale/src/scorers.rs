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
            let completed = completed_calls(&transcript.events);
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
            let calls = completed_calls(&transcript.events);
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

fn completed_calls(events: &[Value]) -> Vec<(String, String, String)> {
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
    let shell_results: std::collections::HashMap<String, String> = events
        .iter()
        .filter(|event| {
            event["type"] == "tool.completed"
                && event.pointer("/data/tool_name").and_then(Value::as_str) == Some("bash")
        })
        .filter_map(|event| {
            Some((
                event.pointer("/data/tool_call_id")?.as_str()?.to_string(),
                event
                    .pointer("/data/result")
                    .map(result_text)
                    .unwrap_or_default(),
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
                if name == "bash" {
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
                let result = data["parent_tool_call_id"]
                    .as_str()
                    .and_then(|id| shell_results.get(id))
                    .cloned()
                    .unwrap_or_default();
                calls.push((name.to_owned(), args, result));
            }
            _ => {}
        }
    }
    calls
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
