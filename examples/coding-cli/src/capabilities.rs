// CLI-owned capabilities for ercode.
//
// These are host/example behavior rather than runtime primitives: `bash` runs
// against the local workspace, and `/model` mutates this process's provider
// selection.

use crate::approval::ApprovalGate;
use crate::runtime::ProviderChoice;
use crate::tools::{BashTool, Workspace};
use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityStatus};
use everruns_core::command::{
    CommandArg, CommandDescriptor, CommandExecutionContext, CommandResult, CommandSource,
    ExecuteCommandRequest,
};
use everruns_core::tools::Tool;
use everruns_runtime::RuntimeProviderStore;
use std::sync::{Arc, RwLock};

// ---------- bash ----------

pub(crate) struct CodingBashCapability {
    pub(crate) workspace: Workspace,
    pub(crate) gate: Arc<ApprovalGate>,
}

impl Capability for CodingBashCapability {
    fn id(&self) -> &str {
        "coding_cli_bash"
    }
    fn name(&self) -> &str {
        "Coding CLI Bash"
    }
    fn description(&self) -> &str {
        "Shell command execution rooted at the host workspace. Requires user approval."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn category(&self) -> Option<&str> {
        Some("Examples")
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        // Harness prompt already documents the `bash` tool. Returning None
        // keeps the capability's contribution out of the system prompt so we
        // don't repeat ourselves.
        None
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BashTool::new(
            self.workspace.clone(),
            self.gate.clone(),
        ))]
    }
}

// ---------- /model ----------
//
// Demonstrates the `Capability::execute_command` hook: `/model` lives entirely
// outside the TUI's `handle_command` branches. The capability owns the runtime
// provider store and shares an Arc<RwLock<ProviderChoice>> with the
// UI-facing `ModelState` so the banner label stays in sync after a switch.

pub(crate) const MODEL_SWITCHER_CAPABILITY_ID: &str = "coding_cli_model_switcher";

pub(crate) struct ModelSwitcherCapability {
    pub(crate) provider: Arc<RwLock<ProviderChoice>>,
    pub(crate) provider_store: Arc<dyn RuntimeProviderStore>,
}

#[async_trait]
impl Capability for ModelSwitcherCapability {
    fn id(&self) -> &str {
        MODEL_SWITCHER_CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Coding CLI Model Switcher"
    }
    fn description(&self) -> &str {
        "Show or change the active provider/model via /model."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn category(&self) -> Option<&str> {
        Some("Examples")
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        None
    }
    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "model".to_string(),
            description: "Show or change the active provider/model.".to_string(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "spec".to_string(),
                description: "<provider>/<id> — omit to print the current model.".to_string(),
                required: false,
                // Declarative completions so renderers can populate the
                // autocomplete dropdown directly from the descriptor — no
                // per-keystroke callback into the capability.
                suggestions: ProviderChoice::model_suggestions()
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            }],
        }]
    }

    async fn execute_command(
        &self,
        request: &ExecuteCommandRequest,
        _ctx: &CommandExecutionContext,
    ) -> everruns_core::Result<CommandResult> {
        if request.name != "model" {
            return Err(everruns_core::AgentLoopError::config(format!(
                "{} cannot execute /{}",
                self.id(),
                request.name
            )));
        }
        let raw = request.arguments.as_deref().unwrap_or("").trim();
        if raw.is_empty() {
            let label = self
                .provider
                .read()
                .expect("provider lock poisoned")
                .label();
            return Ok(CommandResult {
                success: true,
                message: format!(
                    "model: {label}; suggestions: {}",
                    ProviderChoice::model_suggestions().join(", ")
                ),
                error_code: None,
                error_fields: None,
            });
        }

        let current = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .clone();
        let next = match current.resolve_model_spec(raw) {
            Ok(n) => n,
            Err(err) => {
                return Ok(failed_result(format!("model change failed: {err}")));
            }
        };
        let mw = match next.model_with_provider() {
            Ok(m) => m,
            Err(err) => {
                return Ok(failed_result(format!("model change failed: {err}")));
            }
        };
        if let Err(err) = self.provider_store.set_default_model(mw).await {
            return Ok(failed_result(format!("model change failed: {err}")));
        }
        let label = next.label();
        *self.provider.write().expect("provider lock poisoned") = next;
        Ok(CommandResult {
            success: true,
            message: format!("model changed: {label}"),
            error_code: None,
            error_fields: None,
        })
    }
}

fn failed_result(message: String) -> CommandResult {
    CommandResult {
        success: false,
        message,
        error_code: None,
        error_fields: None,
    }
}
