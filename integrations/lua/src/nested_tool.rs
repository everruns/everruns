//! Code mode's host side: a `tools.<name>(args)` call from a Lua script.
//!
//! THREAT[TM-LUA-009]/[TM-TOOL-055]: pre-tool hooks only see the outer `lua`
//! call, so a nested call runs the turn's chains as itself, the way
//! `spawn_background` authorizes its target (EVE-1210). Only the authorized,
//! schema-valid call executes, and the post-tool chain sees (and may rewrite
//! or withhold) its result before the script does. Without the turn's policy
//! nothing is dispatched.

use crate::tool_types::{ToolCall, ToolResult};
use crate::tools::validate_tool_arguments;
use everruns_contracts::runtime::tool_context::ToolContext;
use serde_json::Value;

/// Run one code-mode call; `Err` is the message the script's call raises.
///
/// The child context drops `tool_registry`, so a code-mode tool cannot itself
/// open code mode (no recursion).
pub(crate) async fn call(ctx: &ToolContext, name: &str, args: Value) -> Result<Value, String> {
    let reg = ctx
        .tool_registry
        .as_ref()
        .ok_or_else(|| "tools unavailable in this environment".to_string())?;
    let tool = reg
        .get(name)
        .ok_or_else(|| format!("unknown tool: {name}"))?;
    // The `tools` table only holds eligible names; the host re-checks rather
    // than trusting what the VM asked for.
    if !crate::is_code_mode_eligible(name, &tool.policy(), &tool.hints()) {
        return Err(format!("tool not available in code mode: {name}"));
    }
    let policy = ctx.nested_tool_policy.clone().ok_or_else(|| {
        "code mode requires the turn's tool policy and can only run from an agent turn".to_string()
    })?;
    let tool_def = tool.to_definition();
    let call_id = format!(
        "{}:lua:{name}",
        ctx.tool_call_id.as_deref().unwrap_or("lua")
    );
    let requested = ToolCall {
        id: call_id.clone(),
        name: name.to_string(),
        arguments: args,
    };
    let authorized = policy
        .authorize(requested, &tool_def, ctx)
        .await
        .map_err(refused_message)?;
    // The decision covers this tool; a hook that retargets the call would run
    // something no gate decided on as that tool.
    if authorized.name != name {
        return Err(format!(
            "a pre-tool hook changed the code-mode target from {name} to {}; refusing to run it",
            authorized.name
        ));
    }
    match validate_tool_arguments(tool.as_ref(), &authorized) {
        Ok(None) => {}
        Ok(Some(invalid)) => return Err(invalid),
        Err(_) => return Err("tool internal error".to_string()),
    }

    let mut child = ctx.clone();
    child.tool_registry = None;
    child.tool_call_id = Some(call_id.clone());
    let mut result = tool
        .execute_with_context(authorized.execution_arguments(), &child)
        .await
        .into_tool_result(&call_id, name);
    policy
        .after_exec(&authorized, &tool_def, &mut result, ctx)
        .await;
    if let Some(required) = &result.connection_required {
        return Err(format!("tool requires a connection: {}", required.provider));
    }
    match result.error {
        Some(error) => Err(error),
        None => Ok(result.result.unwrap_or(Value::Null)),
    }
}

/// Script-facing text for a nested call the pre-tool chain did not let run.
///
/// A hosted approval request cannot pause a script mid-run, and its prompt is
/// raised only from a direct call's recorded result, so the script is told to
/// make the call directly rather than to retry it from code mode.
fn refused_message(outcome: ToolResult) -> String {
    let approval_required = outcome
        .result
        .as_ref()
        .and_then(|v| v.get("code"))
        .and_then(|v| v.as_str())
        == Some(everruns_contracts::TOOL_APPROVAL_REQUIRED_CODE);
    if approval_required {
        return "this call needs a person's approval, which code mode cannot request; \
                call the tool directly instead"
            .to_string();
    }
    outcome
        .error
        .unwrap_or_else(|| "blocked by tool policy".to_string())
}

#[cfg(test)]
#[path = "nested_tool_tests.rs"]
pub(crate) mod tests;
