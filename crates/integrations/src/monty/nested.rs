//! Every host effect crosses the turn's existing policy and private effect journal.
use super::{journal, value::MAX_VALUE_BYTES};
use everruns_contracts::{
    runtime::{
        durability::{CodeExecutionJournal, ToolCallClaimResult},
        tool_context::ToolContext,
        tool_surface::NATIVE_ONLY_TOOL_NAMES,
        tools::{Tool, validate_tool_arguments},
    },
    tool_types::{ToolCall, ToolPolicy, ToolResult},
};
use serde_json::{Value, json};

pub(super) fn eligible(tool: &dyn Tool) -> bool {
    !NATIVE_ONLY_TOOL_NAMES.contains(&tool.name())
        && !matches!(tool.name(), "python" | "spawn_background" | "edit_file")
        && tool.policy() != ToolPolicy::ClientSide
        && tool.hints().stays_direct != Some(true)
}

fn visible(ctx: &ToolContext, name: &str) -> bool {
    // An absent turn allowlist is not authority to enumerate a registry superset.
    ctx.visible_tool_names
        .as_ref()
        .is_some_and(|names| names.contains(name))
}

pub(super) fn catalog(ctx: &ToolContext, query: &str) -> Result<Value, String> {
    let registry = ctx
        .tool_registry
        .as_ref()
        .ok_or("tool registry unavailable")?;
    let mut names = registry.tool_names();
    names.sort();
    let mut entries = Vec::new();
    let mut used = 0;
    let mut truncated = false;
    for name in names {
        if !visible(ctx, name) || !name.contains(query) {
            continue;
        }
        let Some(tool) = registry.get(name) else {
            continue;
        };
        if !eligible(tool.as_ref()) {
            continue;
        }
        let entry = json!({"name":name,"description":tool.description(),"parameters":tool.parameters_schema()});
        let Ok(size) = super::value::encoded_size(&entry, MAX_VALUE_BYTES / 2 - used) else {
            truncated = true;
            continue;
        };
        used += size;
        entries.push(entry);
    }
    Ok(
        json!({"tools":entries,"truncated":truncated,"hint":"Use tools('name_fragment') to narrow the catalog. Direct-only tools remain available outside Python."}),
    )
}

#[derive(Debug)]
pub(super) enum Outcome {
    Result(Value),
    Pause(ToolResult),
}

pub(super) async fn invoke(
    ctx: &ToolContext,
    journal: &CodeExecutionJournal,
    key: &str,
    name: &str,
    arguments: Value,
) -> Result<Outcome, String> {
    if !visible(ctx, name) {
        return Err(format!("tool not available in this turn: {name}"));
    }
    let registry = ctx
        .tool_registry
        .as_ref()
        .ok_or("tool registry unavailable")?;
    let tool = registry.get(name).ok_or("unknown tool")?;
    if !eligible(tool.as_ref()) {
        return Err(format!("{name} must be called directly outside Python"));
    }
    if let Some(checker) = &ctx.budget_checker {
        let budget = checker
            .check_budgets(&ctx.session_id.to_string())
            .await
            .map_err(|_| "Cannot verify the execution budget")?;
        if matches!(budget.status.as_str(), "paused" | "exhausted") {
            return Err("Execution budget does not permit another nested tool call".into());
        }
    }
    let policy = ctx
        .nested_tool_policy
        .as_ref()
        .ok_or("Python tool calls require the turn's policy")?;
    let definition = tool.to_definition();
    let call = ToolCall {
        id: key.to_owned(),
        name: name.to_owned(),
        arguments,
    };
    let mut child = ctx.clone();
    child.tool_call_id = Some(key.to_owned());
    child.idempotency_key = Some(format!("{}:{key}", journal.namespace));
    // A nested executor cannot open a second execution surface.
    child.tool_registry = None;
    // Some integrations have a standalone direct transport. Code mode must
    // never select it merely because its host omitted an egress service.
    child.egress_service.get_or_insert_with(|| {
        std::sync::Arc::new(everruns_contracts::runtime::egress::DisabledEgressService)
    });
    let started = std::time::Instant::now();
    let authorized = match policy.authorize(call, &definition, &child).await {
        Ok(call) => call,
        Err(result) => {
            let status =
                if everruns_contracts::tool_types::ToolApprovalRequired::from_tool_result(&result)
                    .is_some()
                {
                    "needs_approval"
                } else {
                    "refused"
                };
            record(
                ctx,
                key,
                name,
                &call_arguments_for_preview(&result),
                status,
                started.elapsed(),
            )
            .await;
            return Ok(Outcome::Pause(result));
        }
    };
    if authorized.name != name || authorized.id != key {
        return Err("pre-tool hook changed the nested tool identity; refusing execution".into());
    }
    if let Some(error) =
        validate_tool_arguments(tool.as_ref(), &authorized).map_err(|_| "invalid tool schema")?
    {
        return Err(error);
    }
    let fingerprint = journal::digest(
        serde_json::to_vec(&json!({"tool":name,"arguments":authorized.execution_arguments()}))
            .map_err(|_| "invalid tool arguments")?,
    );
    let claim = journal
        .store
        .try_claim_tool_call(&journal.namespace, key, name, &fingerprint)
        .await
        .map_err(|_| "Python effect journal unavailable")?;
    let mut result = match claim {
        ToolCallClaimResult::AlreadySettled {
            result_json,
            args_fingerprint,
        } if args_fingerprint == fingerprint => {
            serde_json::from_value::<ToolResult>(result_json)
                .map_err(|_| "invalid recorded Python tool result")?
        }
        ToolCallClaimResult::Claimed { claim_token } => {
            let mut result = tool
                .execute_with_context(authorized.execution_arguments(), &child)
                .await
                .into_tool_result(key, name);
            policy
                .after_exec(&authorized, &definition, &mut result, &child)
                .await;
            // Bounded, policy-filtered result only. Raw output and images have
            // native artifact semantics and must not cross this JSON bridge.
            result.raw_output = None;
            result.details = None;
            if result.images.is_some() || result.connection_required.is_some() {
                result.images = None;
                result.connection_required = None;
                result.error =
                    Some("This result requires a native tool call; call the tool directly".into());
                result.result = None;
            }
            if super::value::encoded_size(&result, MAX_VALUE_BYTES).is_err() {
                result.result = None;
                result.error = Some(
                    "Tool result exceeds Python's 256 KiB bridge limit; request a smaller result"
                        .into(),
                );
            }
            let saved = serde_json::to_value(&result).map_err(|_| "invalid tool result")?;
            if !journal.store.settle_tool_call(&journal.namespace, key, saved, "settled", claim_token).await.map_err(|_| "effect may have completed but its result was not recorded; do not repeat it")? {
                return Err("effect journal ownership lost; do not repeat the operation".into());
            }
            record(
                ctx,
                key,
                name,
                &authorized.arguments,
                if result.error.is_some() {
                    "failed"
                } else {
                    "completed"
                },
                started.elapsed(),
            )
            .await;
            return Ok(Outcome::Result(
                json!({"result":result.result,"error":result.error}),
            ));
        }
        ToolCallClaimResult::AlreadyRunning { .. } => {
            return Err(
                "nested tool outcome is unknown after interruption; refusing to repeat its effect"
                    .into(),
            );
        }
        _ => return Err("nested tool arguments changed during replay; refusing execution".into()),
    };
    // Replayed results still pass current output policy before entering Python.
    policy
        .after_exec(&authorized, &definition, &mut result, &child)
        .await;
    Ok(Outcome::Result(
        json!({"result":result.result,"error":result.error}),
    ))
}

fn call_arguments_for_preview(result: &ToolResult) -> Value {
    result
        .result
        .as_ref()
        .and_then(|v| v.get("arguments"))
        .cloned()
        .unwrap_or(Value::Null)
}
async fn record(
    ctx: &ToolContext,
    key: &str,
    name: &str,
    input: &Value,
    status: &str,
    duration: std::time::Duration,
) {
    use crate::code_mode_timeline::{self, Call, Status};
    code_mode_timeline::record(
        ctx,
        Call {
            call_id: key,
            tool_name: name,
            command: &format!("tool({name:?}, ...)"),
            input,
            status: match status {
                "completed" => Status::Completed,
                "needs_approval" => Status::NeedsApproval,
                "refused" => Status::Refused("tool policy refused execution"),
                _ => Status::Failed("nested tool failed"),
            },
            duration: Some(duration),
        },
    )
    .await;
}
