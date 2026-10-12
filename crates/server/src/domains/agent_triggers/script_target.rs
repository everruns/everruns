//! Triggers that run a saved script instead of asking the model (Tools in
//! Shell D9, `knowledge/execution/tools-in-shell.md`).
//!
//! Decisions:
//! - Only schedule and webhook triggers take a script, as the design names;
//!   GitHub and MCP event triggers keep sending their message.
//! - The trigger still renders its message: it is the user message the run
//!   is recorded under, so the session shows why the script ran.
//! - The script must exist when the trigger is saved. A script archived later
//!   makes the run fail with the shell's own "no saved script" result, which
//!   is recorded like any other failed run.
//! - Input strings are templates over the event, as the message is. A string
//!   that is only one `{{path}}` takes the value at that path as is (an
//!   object stays an object), so `{"pr": "{{payload.pull_request}}"}` hands
//!   the script the webhook's pull request; any other string is rendered as
//!   text. The input is never sent as text through the shell: it reaches the
//!   script as JSON on stdin.

use everruns_contracts::runtime::saved_scripts::{ScriptRun, is_valid_script_name};
use serde_json::Value;

use crate::domains::agent_channels::invocation::{render_message_template, template_lookup};

use crate::domains::common::*;
use crate::storage::AgentRow;

/// Check `run` against the agent's saved scripts.
pub(super) async fn validate(
    ctx: &Ctx,
    agent: &AgentRow,
    run: &ScriptRun,
) -> Result<(), CommandError> {
    if !is_valid_script_name(&run.script) {
        return Err(CommandError::bad_request(format!(
            "'{}' is not a saved script name",
            run.script
        )));
    }
    if run.input.as_ref().is_some_and(|input| !input.is_object()) {
        return Err(CommandError::bad_request(
            "script input must be a JSON object",
        ));
    }
    let scripts = ctx
        .db
        .list_agent_scripts(ctx.org_id(), agent.id, false)
        .await?;
    if !scripts.iter().any(|script| script.name == run.script) {
        return Err(CommandError::bad_request(format!(
            "Agent has no saved script named '{}'",
            run.script
        )));
    }
    Ok(())
}

/// The script a trigger runs after an update: a request with an empty script
/// name clears it, no script in the request keeps the stored one.
pub(super) async fn updated(
    ctx: &Ctx,
    agent: &AgentRow,
    requested: Option<ScriptRun>,
    stored: Option<ScriptRun>,
) -> Result<Option<ScriptRun>, CommandError> {
    match requested {
        None => Ok(stored),
        Some(run) if run.script.is_empty() => Ok(None),
        Some(run) => {
            validate(ctx, agent, &run).await?;
            Ok(Some(run))
        }
    }
}

/// `run` with its input rendered against the event's template context.
pub(super) fn rendered(run: &ScriptRun, context: &Value) -> ScriptRun {
    ScriptRun {
        input: run.input.as_ref().map(|input| render_input(input, context)),
        ..run.clone()
    }
}

fn render_input(value: &Value, context: &Value) -> Value {
    match value {
        Value::String(text) => match whole_placeholder(text) {
            Some(path) => template_lookup(context, path)
                .cloned()
                .unwrap_or(Value::Null),
            None => Value::String(render_message_template(text, context)),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_input(item, context))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), render_input(item, context)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The path of a string that is exactly one `{{path}}` placeholder.
fn whole_placeholder(text: &str) -> Option<&str> {
    let inner = text.trim().strip_prefix("{{")?.strip_suffix("}}")?.trim();
    let valid = !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    valid.then_some(inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn input_strings_render_against_the_event() {
        let context = json!({
            "payload": {"number": 7, "pull_request": {"title": "Fix", "draft": false}},
            "trigger": {"name": "prs"}
        });
        let run = ScriptRun {
            script: "triage".into(),
            input: Some(json!({
                "pr": "{{payload.pull_request}}",
                "number": "{{ payload.number }}",
                "label": "#{{payload.number}} from {{trigger.name}}",
                "missing": "{{payload.nope}}",
                "fixed": 3,
                "list": ["{{payload.pull_request.draft}}"]
            })),
            wake_agent_on_failure: false,
        };
        let input = rendered(&run, &context).input.unwrap();
        assert_eq!(
            input,
            json!({
                "pr": {"title": "Fix", "draft": false},
                "number": 7,
                "label": "#7 from prs",
                "missing": null,
                "fixed": 3,
                "list": [false]
            })
        );
    }

    #[test]
    fn only_a_lone_placeholder_keeps_its_value() {
        assert_eq!(whole_placeholder("{{a.b}}"), Some("a.b"));
        assert_eq!(whole_placeholder(" {{ a }} "), Some("a"));
        assert_eq!(whole_placeholder("{{a}} {{b}}"), None);
        assert_eq!(whole_placeholder("x{{a}}"), None);
        assert_eq!(whole_placeholder("{{}}"), None);
    }
}
