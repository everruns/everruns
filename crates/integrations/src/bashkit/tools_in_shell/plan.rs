//! `tools plan`: what a script would call, and how risky each call is,
//! without running anything.
//!
//! Decisions:
//!
//! - **Same reading as the early stop.** The plan lists the `tools` calls
//!   Bashkit's analysis can see and asks the pre-tool chain about each one
//!   exactly as the early stop does (`preview`: nobody is asked and no answer
//!   is used up), so a call the plan marks `needs_approval` is the call that
//!   would hold the script.
//! - **Honest about what it cannot see.** A call whose input is built at run
//!   time is listed with `input: null` and judged on the tool alone; a script
//!   that names commands at run time or uses `eval` says so in `complete`.
//! - **For soft approval.** The agent runs it to describe a script before
//!   asking a person about it; it never decides anything itself.

use bashkit::{CommandContext, ScriptAnalysis};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::tool_types::ToolCall;
use serde_json::{Value, json};

use super::TOOLS_COMMAND;
use super::catalog::Catalog;
use super::preflight::visible_call;

/// The plan for `script`, as the JSON `tools plan` prints.
pub(super) async fn plan(context: &ToolContext, catalog: &Catalog, script: &str) -> Value {
    let analysis = match bashkit::Bash::new().analyze(script) {
        Ok(analysis) => analysis,
        Err(failure) => {
            return json!({"error": format!("the script does not parse: {failure}")});
        }
    };
    let mut calls = Vec::new();
    for command in analysis
        .commands
        .iter()
        .filter(|c| c.name.as_deref() == Some(TOOLS_COMMAND))
    {
        let Some(call) = planned_call(context, catalog, command).await else {
            continue;
        };
        calls.push(call);
    }
    json!({
        "calls": calls,
        "complete": is_complete(&analysis),
    })
}

fn is_complete(analysis: &ScriptAnalysis) -> bool {
    !analysis.is_opaque() && analysis.command_wrappers().is_empty()
}

async fn planned_call(
    context: &ToolContext,
    catalog: &Catalog,
    command: &bashkit::AnalyzedCommand,
) -> Option<Value> {
    let place = match command.context {
        CommandContext::Direct => "script",
        CommandContext::Substitution => "substitution",
        CommandContext::FunctionBody => "function",
    };
    let literal: Option<Vec<String>> = command
        .literal_args()
        .map(|args| args.into_iter().map(str::to_string).collect());
    // A call with input on the line, fully literal: judged as that exact call.
    if let Some(args) = &literal
        && let Some((entry, input)) = visible_call(catalog, args)
    {
        let risk = exact_risk(context, &entry, &input).await;
        return Some(json!({
            "tool": entry.command_line(),
            "input": input,
            "risk": risk,
            "where": place,
        }));
    }
    // Otherwise name the tool when its words are literal, and judge the tool.
    let words: Vec<&str> = command
        .args
        .iter()
        .take(2)
        .map_while(|a| a.as_deref())
        .collect();
    let entry = match words.as_slice() {
        [source, name, ..] if catalog.is_source(source) => catalog.in_source(source, name),
        [name, ..] => catalog.top_level(name),
        [] => None,
    };
    let Some(entry) = entry else {
        return Some(json!({
            "tool": null,
            "input": null,
            "risk": "checked_at_run_time",
            "where": place,
        }));
    };
    let risk = if entry.tool.hints().readonly == Some(true) {
        "read_only"
    } else {
        "checked_at_run_time"
    };
    Some(json!({
        "tool": entry.command_line(),
        "input": null,
        "risk": risk,
        "where": place,
    }))
}

async fn exact_risk(
    context: &ToolContext,
    entry: &super::catalog::Entry,
    input: &Value,
) -> &'static str {
    let read_only = entry.tool.hints().readonly == Some(true);
    let Some(policy) = context.nested_tool_policy.as_ref() else {
        return if read_only { "read_only" } else { "changes" };
    };
    let call = ToolCall {
        id: format!(
            "{}:tools:plan:{}",
            context.tool_call_id.as_deref().unwrap_or("bash"),
            entry.tool_name
        ),
        name: entry.tool_name.clone(),
        arguments: input.clone(),
    };
    // The rated definition, so a tool without hints that the decision service
    // judges to change things shows `needs_approval` here as it would at run time.
    let definition = super::ratings::definition(context, entry).await;
    match policy.preview(&call, &definition, context).await {
        Some(held) => {
            let approval = held
                .result
                .as_ref()
                .and_then(|r| r.get("code"))
                .and_then(Value::as_str)
                == Some(everruns_contracts::TOOL_APPROVAL_REQUIRED_CODE);
            if approval {
                "needs_approval"
            } else {
                "blocked"
            }
        }
        None if read_only => "read_only",
        None => "changes",
    }
}
