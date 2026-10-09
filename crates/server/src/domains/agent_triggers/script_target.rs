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

use everruns_contracts::runtime::saved_scripts::{ScriptRun, is_valid_script_name};

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
