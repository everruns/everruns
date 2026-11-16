//! Session-only declarations, strict dispatch and model-safe results.
use super::*;

pub(super) fn declarations() -> Vec<Declaration> {
    let mut tools = vec![
        Declaration {
            name: STATUS_TOOL,
            description: "Check personal browser connection. No pairing secrets are returned.",
            parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
        },
        Declaration {
            name: TABS_TOOL,
            description: "List tabs lent or owned through the personal browser. Does not open or close tabs.",
            parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
        },
        Declaration {
            name: HANDOFF_TOOL,
            description: "Ask the user to act in an owned or lent personal-browser tab and wait for genuine Continue in the browser extension (stopping, timeout and closed-tab outcomes are distinct). Re-observe the page afterward.",
            parameters: json!({"type":"object","properties":{"tab":{"type":"integer","minimum":1},"message":{"type":"string","minLength":1,"maxLength":4096},"timeoutSeconds":{"type":"integer","minimum":1,"maximum":600}},"required":["tab","message"],"additionalProperties":false}),
        },
    ];
    tools.extend(actions::declarations());
    tools
}
pub(super) async fn configure(
    engine: &Engine,
    session: SessionId,
    enabled: bool,
) -> Result<(), CoordinatorError> {
    let profile = engine.profile().map_err(|_| CoordinatorError::Authority)?;
    let snapshot = profile
        .query_snapshot(session)
        .await
        .map_err(|_| CoordinatorError::Authority)?;
    let mut tools = snapshot.configuration.client_tools.unwrap_or_else(|| {
        engine
            .agent(session)
            .map(|agent| agent.declared_client_tools())
            .unwrap_or_default()
    });
    let declared = declarations();
    for tool in &tools {
        if let Some(expected) = declared
            .iter()
            .find(|expected| expected.name == tool.name())
            && (tool.parameters() != &expected.parameters
                || tool.description() != expected.description)
        {
            return Err(CoordinatorError::InvalidConfiguration);
        }
    }
    tools.retain(|tool| !owns(tool.name()));
    if enabled {
        tools.extend(declarations().into_iter().map(|tool| {
            ToolDefinition::ClientSide(ClientSideTool::new(
                tool.name,
                tool.description,
                tool.parameters,
            ))
        }));
    }
    profile
        .configure(
            session,
            Uuid::new_v4(),
            SessionConfiguration {
                client_tools: Some(tools),
                ..Default::default()
            },
        )
        .await
        .map_err(|_| CoordinatorError::Authority)?;
    Ok(())
}
pub(super) fn action_timeout(call: &ToolCall) -> Duration {
    Duration::from_secs(if call.name == HANDOFF_TOOL {
        call.arguments["timeoutSeconds"]
            .as_u64()
            .unwrap_or(600)
            .clamp(1, 600)
            + 5
    } else {
        60
    })
}
pub(super) async fn execute(
    supervisor: &BrowseSupervisor,
    session: SessionId,
    call: &ToolCall,
) -> Result<Value, BrowseError> {
    let args = call.execution_arguments();
    let object = args.as_object().ok_or(BrowseError::InvalidRequest)?;
    match call.name.as_str() {
        STATUS_TOOL | TABS_TOOL if !object.is_empty() => Err(BrowseError::InvalidRequest),
        STATUS_TOOL => {
            let result = supervisor
                .call_tool(session.uuid(), "status", json!({}))
                .await?;
            let data = tool_json(&result)?;
            // Never forward upstream extension diagnostics, endpoints, pairing
            // links, tokens, update commands or free-form nextStep to the model.
            Ok(
                json!({"connected":data["connected"].as_bool().ok_or(BrowseError::ProtocolError)?,"nextStep":"Use the trusted Browse extension to pair or lend a tab if needed."}),
            )
        }
        TABS_TOOL => {
            let result = supervisor
                .call_tool(session.uuid(), "tabs", json!({"action":"list"}))
                .await?;
            let data = tool_json(&result)?;
            let tabs = data["tabs"]
                .as_array()
                .ok_or(BrowseError::ProtocolError)?
                .iter()
                .map(|tab| {
                    let mut clean = serde_json::Map::new();
                    for key in ["tab", "id", "title", "url", "lent", "group"] {
                        if let Some(value) = tab.get(key) {
                            clean.insert(key.into(), value.clone());
                        }
                    }
                    Value::Object(clean)
                })
                .collect::<Vec<_>>();
            Ok(json!({"tabs":tabs}))
        }
        HANDOFF_TOOL => {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "tab" | "message" | "timeoutSeconds"))
            {
                return Err(BrowseError::InvalidRequest);
            }
            let tab = args["tab"]
                .as_u64()
                .filter(|tab| *tab > 0)
                .ok_or(BrowseError::InvalidRequest)?;
            let message = args["message"]
                .as_str()
                .ok_or(BrowseError::InvalidRequest)?;
            let timeout = match args.get("timeoutSeconds") {
                None => 600,
                Some(value) => value
                    .as_u64()
                    .filter(|seconds| (1..=600).contains(seconds))
                    .ok_or(BrowseError::InvalidRequest)?,
            };
            let outcome = supervisor
                .handoff(session.uuid(), tab, message, Duration::from_secs(timeout))
                .await?;
            match outcome {
                HandoffOutcome::Continued => {
                    Ok(json!({"outcome":"continued","tab":tab,"reobserve":true}))
                }
                HandoffOutcome::Cancelled => Ok(json!({"outcome":"cancelled","tab":tab})),
                HandoffOutcome::TimedOut => Ok(json!({"outcome":"timeout","tab":tab})),
                HandoffOutcome::TabClosed => Ok(json!({"outcome":"closed","tab":tab})),
            }
        }
        name if actions::upstream(name).is_some() => {
            actions::validate(name, &args)?;
            let result = supervisor
                .call_tool(
                    session.uuid(),
                    actions::upstream(name).ok_or(BrowseError::InvalidRequest)?,
                    args,
                )
                .await?;
            actions::output(name, result)
        }
        _ => Err(BrowseError::InvalidRequest),
    }
}
fn tool_json(result: &Value) -> Result<Value, BrowseError> {
    match result.get("isError") {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => return Err(BrowseError::RemoteError),
        _ => return Err(BrowseError::ProtocolError),
    }
    for content in result["content"]
        .as_array()
        .ok_or(BrowseError::ProtocolError)?
    {
        if content["type"] == "text"
            && let Some(text) = content["text"].as_str()
            && let Ok(value) = serde_json::from_str(text)
        {
            return Ok(value);
        }
    }
    Err(BrowseError::ProtocolError)
}

/// Preserve failed human outcomes and bound the complete authority payload.
pub(super) fn completed(
    call: &ToolCall,
    outcome: Result<Value, BrowseError>,
) -> everruns_core::events::ToolCompletedData {
    let mut result = match outcome {
        Ok(value) if call.name == actions::SCREENSHOT => actions::completed(call, &value),
        Ok(value) if call.name == HANDOFF_TOOL && value["outcome"] != "continued" => {
            let outcome = value["outcome"].as_str().unwrap_or("unknown");
            let status = match outcome {
                "cancelled" => "cancelled",
                "timeout" => "timeout",
                _ => "error",
            };
            let mut failed = everruns_core::events::ToolCompletedData::failure(
                call.id.clone(),
                call.name.clone(),
                status.into(),
                format!("Personal browser handoff {outcome}; human continuation was not received."),
                None,
            );
            failed.details = Some(value);
            failed
        }
        Ok(value) => ToolAnswer {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            result: value,
            error: None,
        }
        .into_event_data(),
        Err(_) => ToolAnswer {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            result: Value::Null,
            error: Some(
                "Personal browser operation did not complete. Do not replay an unknown mutation."
                    .into(),
            ),
        }
        .into_event_data(),
    };
    if serde_json::to_vec(&result).map_or(true, |bytes| bytes.len() > 900_000) {
        result=ToolAnswer{tool_call_id:call.id.clone(),tool_name:call.name.clone(),result:Value::Null,error:Some("Browser result exceeds the durable result size limit; use a text snapshot instead.".into())}.into_event_data();
    }
    result
}
