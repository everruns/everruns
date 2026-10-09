//! Stop a script before it starts when a `tools` call in it needs approval.
//!
//! Decisions:
//!
//! - **Only what analysis can see.** Bashkit's parser lists the script's
//!   commands without running it. A `tools` call is previewed only when its
//!   command and every argument are literal, it runs in the script itself (not
//!   in a function or a substitution), and it does not read its input from
//!   stdin. Anything else is left to the run-time stop, which catches every
//!   call however it was built.
//! - **Asked as the call itself.** The preview asks the turn's pre-tool chain
//!   with the exact tool and input, so the request the person sees, and the
//!   one-off answer they give, bind to the call the script makes. Previewing
//!   asks nobody and uses up no answer.
//! - **Nothing ran, so the same script may run again.** Unlike a stop part way
//!   through, a script held here did nothing, and the report says so.

use bashkit::ScriptAnalysis;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::tool_types::ToolCall;
use serde_json::{Value, json};

use super::TOOLS_COMMAND;
use super::builtin::MAX_CALLS_PER_EXECUTION;
use super::catalog::{Catalog, Entry, strip_human_intent};
use super::input::{self, Request};
use super::run::{Run, StopReason};

/// The `bash` result for a script held before it starts, or `None` to run it.
/// `analysis` is the shell's own reading of the script; `None` (it did not
/// parse) leaves everything to the run.
pub async fn preflight(context: &ToolContext, analysis: Option<ScriptAnalysis>) -> Option<Value> {
    let policy = context.nested_tool_policy.as_ref()?;
    if !super::installs(context) {
        return None;
    }
    let analysis = analysis?;
    let catalog = Catalog::from_context(context);
    let calls = analysis
        .commands
        .iter()
        .filter(|c| c.name.as_deref() == Some(TOOLS_COMMAND))
        .filter(|c| c.context == bashkit::CommandContext::Direct)
        .filter_map(|c| c.literal_args())
        .take(MAX_CALLS_PER_EXECUTION);
    for args in calls {
        let args: Vec<String> = args.into_iter().map(str::to_string).collect();
        let Some((entry, input)) = visible_call(&catalog, &args) else {
            continue;
        };
        let call = ToolCall {
            id: format!(
                "{}:tools:preflight:{}",
                context.tool_call_id.as_deref().unwrap_or("bash"),
                entry.tool_name
            ),
            name: entry.tool_name.clone(),
            arguments: input.clone(),
        };
        let definition = super::ratings::definition(context, &entry).await;
        let held = policy.preview(&call, &definition, context).await;
        let approval = held.and_then(|held| held.result).filter(|payload| {
            payload.get("code").and_then(Value::as_str)
                == Some(everruns_contracts::TOOL_APPROVAL_REQUIRED_CODE)
        });
        if let Some(approval) = approval {
            return Some(held_result(&entry.command_line(), &input, approval));
        }
    }
    None
}

/// The tool and input a literal `tools ...` line calls, when it calls one
/// with input fully written on the line.
pub(super) fn visible_call(catalog: &Catalog, args: &[String]) -> Option<(Entry, Value)> {
    let first = args.first()?;
    let (entry, rest) = if catalog.is_source(first) {
        (catalog.in_source(first, args.get(1)?)?.clone(), &args[2..])
    } else {
        (catalog.top_level(first)?.clone(), &args[1..])
    };
    if rest.iter().any(|a| a == "-") {
        return None;
    }
    let schema = strip_human_intent(entry.tool.parameters_schema());
    match input::parse(rest, None, &schema) {
        Ok(Request::Call(input)) => Some((entry, input)),
        _ => None,
    }
}

fn held_result(command: &str, input: &Value, approval: Value) -> Value {
    let mut result = json!({
        "stdout": "",
        "stderr": format!(
            "{command} needs a person's approval, so the script did not start. The person is \
             being asked; after they answer, run it again.\n"
        ),
        "exit_code": 1,
        "success": false,
        "truncated": false,
        "total_lines": 1,
    });
    let run = Run::default();
    run.stop(StopReason::NeedsApproval, command, input, Some(approval));
    run.apply(&mut result);
    result["tools"]["stopped"]["before_start"] = json!(true);
    result["tools"]["not_reached"] = json!("the whole script: nothing ran");
    result
}
