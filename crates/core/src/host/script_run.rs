//! Script runs (Tools in Shell D9): a turn that runs one saved script with no
//! model call.
//!
//! Decisions:
//! - A script run is an ordinary turn whose model is replaced by
//!   [`ScriptRunDriver`]. The turn still goes reason → act → reason, so the
//!   `bash` call, its approval gate, its events and its transcript are exactly
//!   the ones a model-made call gets; nothing in the turn engine knows about
//!   script runs.
//! - The first reason answers with one `bash` call that runs the script. The
//!   second ends the turn with a fixed line, or, when the run failed or stopped
//!   and the trigger asked for it, hands the transcript to the agent's real
//!   model, which answers in the same turn ("wake the agent").
//! - Only the platform can start one: the marker is a reserved message
//!   metadata key that client metadata loses.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent, Message,
    MessageRole,
};
use everruns_contracts::error::Result;
use everruns_contracts::runtime::saved_scripts::{ScriptRun, is_valid_script_name};
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::tool_types::ToolCall;
use serde_json::Value;

use crate::message::{RuntimeMessage, RuntimeMessageRole};

/// The script run the turn's latest user message asks for, if any, with the
/// id of the one tool call it makes. The id names the message that asked, so
/// the second reason finds this run's call and never an earlier run's.
pub(crate) fn requested(messages: &[RuntimeMessage]) -> Option<(ScriptRun, String)> {
    let latest = messages
        .iter()
        .rev()
        .find(|message| message.role == RuntimeMessageRole::User)?;
    let run = ScriptRun::from_metadata(latest.metadata.as_ref())?;
    Some((run, format!("script_run_{}", latest.id.uuid().simple())))
}

/// Stands in for the agent's model for one script run.
pub(crate) struct ScriptRunDriver {
    run: ScriptRun,
    call_id: String,
    /// The agent's real model, used only to wake the agent.
    model: Arc<dyn ChatDriver>,
}

impl ScriptRunDriver {
    pub(crate) fn new(run: ScriptRun, call_id: String, model: Arc<dyn ChatDriver>) -> Self {
        Self {
            run,
            call_id,
            model,
        }
    }

    fn step(&self, messages: &[Message], config: &LlmCallConfig) -> Step {
        let call = messages.iter().rposition(|message| {
            message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| calls.iter().any(|call| call.id == self.call_id))
        });
        let Some(call) = call else {
            if !is_valid_script_name(&self.run.script) {
                return Step::Finish(format!(
                    "Script run skipped: `{}` is not a saved script name.",
                    self.run.script
                ));
            }
            if !config.tools.iter().any(|tool| tool.name() == "bash") {
                return Step::Finish(format!(
                    "Saved script `{}` did not run: this agent has no bash tool. \
                     Enable Tools in Shell to run saved scripts.",
                    self.run.script
                ));
            }
            return Step::Call;
        };
        let after = &messages[call + 1..];
        if after
            .iter()
            .any(|message| message.role == MessageRole::Assistant)
        {
            // The agent was woken and is answering: it owns the rest.
            return Step::Model;
        }
        let result = after.iter().find(|message| {
            message.role == MessageRole::Tool
                && message.tool_call_id.as_deref() == Some(self.call_id.as_str())
        });
        let succeeded = result.is_some_and(|message| ran_clean(&message.content.to_text()));
        if succeeded {
            Step::Finish(format!("Ran saved script `{}`.", self.run.script))
        } else if self.run.wake_agent_on_failure {
            Step::Model
        } else {
            Step::Finish(format!(
                "Saved script `{}` did not finish; its result above says why.",
                self.run.script
            ))
        }
    }
}

enum Step {
    /// Make the `bash` call that runs the script.
    Call,
    /// End the turn with this line.
    Finish(String),
    /// Hand the turn to the agent's model.
    Model,
}

/// Whether a `bash` result is a run that finished with exit 0 and no stop.
fn ran_clean(result: &str) -> bool {
    let Ok(result) = serde_json::from_str::<Value>(result) else {
        return false;
    };
    result.get("exit_code").and_then(Value::as_i64) == Some(0)
        && result.get("success").and_then(Value::as_bool) != Some(false)
}

#[async_trait]
impl ChatDriver for ScriptRunDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let event = match self.step(&messages, config) {
            Step::Model => {
                return self
                    .model
                    .chat_completion_stream(endpoint, messages, config)
                    .await;
            }
            Step::Call => LlmStreamEvent::ToolCalls(vec![ToolCall {
                id: self.call_id.clone(),
                name: "bash".to_string(),
                arguments: serde_json::json!({ "commands": self.run.command() }),
            }]),
            Step::Finish(text) => LlmStreamEvent::TextDelta(text),
        };
        let done = LlmStreamEvent::Done(Box::new(LlmCompletionMetadata::default()));
        Ok(Box::pin(futures::stream::iter([Ok(event), Ok(done)])))
    }

    fn effective_context_window(&self, model: &str) -> Option<usize> {
        self.model.effective_context_window(model)
    }
}

#[cfg(test)]
#[path = "script_run_tests.rs"]
mod tests;
