// CLI-owned capabilities for ercode.
//
// These are host/example behavior rather than runtime primitives: `bash` runs
// against the local workspace, and `/model` mutates this process's provider
// selection.

use crate::approval::ApprovalGate;
use crate::runtime::ProviderChoice;
use crate::tools::{BashTool, Workspace};
use async_trait::async_trait;
use chrono::Local;
use everruns_core::capabilities::{Capability, CapabilityStatus, SystemPromptContext};
use everruns_core::command::{
    CommandArg, CommandDescriptor, CommandExecutionContext, CommandResult, CommandSource,
    ExecuteCommandRequest,
};
use everruns_core::tools::Tool;
use everruns_runtime::RuntimeProviderStore;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, RwLock};

// ---------- environment context ----------

pub(crate) const ENVIRONMENT_CONTEXT_CAPABILITY_ID: &str = "coding_cli_environment_context";

pub(crate) struct CodingCliEnvironmentCapability {
    pub(crate) workspace_root: PathBuf,
}

#[async_trait]
impl Capability for CodingCliEnvironmentCapability {
    fn id(&self) -> &str {
        ENVIRONMENT_CONTEXT_CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Coding CLI Environment Context"
    }
    fn description(&self) -> &str {
        "Adds current workspace, shell, date, timezone, and Git context to the prompt."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn category(&self) -> Option<&str> {
        Some("Examples")
    }
    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        Some(render_environment_context(&EnvironmentContext::collect(
            &self.workspace_root,
        )))
    }
    fn system_prompt_preview(&self) -> Option<String> {
        Some(render_environment_context(&EnvironmentContext::collect(
            &self.workspace_root,
        )))
    }
}

#[derive(Debug)]
struct EnvironmentContext {
    cwd: String,
    shell: String,
    current_date: String,
    timezone: String,
    git_repo: Option<String>,
    git_user: Option<String>,
    git_email: Option<String>,
    git_current_branch: Option<String>,
}

impl EnvironmentContext {
    fn collect(workspace_root: &Path) -> Self {
        Self {
            cwd: workspace_root.display().to_string(),
            shell: shell_name(),
            current_date: Local::now().format("%Y-%m-%d").to_string(),
            timezone: local_timezone(),
            git_repo: git_output(workspace_root, &["config", "--get", "remote.origin.url"])
                .or_else(|| git_output(workspace_root, &["rev-parse", "--show-toplevel"])),
            git_user: git_output(workspace_root, &["config", "--get", "user.name"]),
            git_email: git_output(workspace_root, &["config", "--get", "user.email"]),
            git_current_branch: git_current_branch(workspace_root),
        }
    }
}

fn render_environment_context(context: &EnvironmentContext) -> String {
    let mut out = String::new();
    out.push_str("<environment_context>\n");
    push_xml_field(&mut out, "cwd", &context.cwd);
    push_xml_field(&mut out, "shell", &context.shell);
    push_xml_field(&mut out, "current_date", &context.current_date);
    push_xml_field(&mut out, "timezone", &context.timezone);
    if let Some(value) = &context.git_repo {
        push_xml_field(&mut out, "git_repo", value);
    }
    if let Some(value) = &context.git_user {
        push_xml_field(&mut out, "git_user", value);
    }
    if let Some(value) = &context.git_email {
        push_xml_field(&mut out, "git_email", value);
    }
    if let Some(value) = &context.git_current_branch {
        push_xml_field(&mut out, "git_current_branch", value);
    }
    out.push_str("</environment_context>");
    out
}

fn push_xml_field(out: &mut String, name: &str, value: &str) {
    out.push_str("  <");
    out.push_str(name);
    out.push('>');
    out.push_str(&xml_escape(value));
    out.push_str("</");
    out.push_str(name);
    out.push_str(">\n");
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn shell_name() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|shell| {
            Path::new(&shell)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "sh".to_string())
}

fn local_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ")
        && !tz.trim().is_empty()
    {
        return tz;
    }
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let target = target.to_string_lossy();
        if let Some((_, timezone)) = target.split_once("/zoneinfo/") {
            return timezone.to_string();
        }
    }
    "local".to_string()
}

fn git_current_branch(workspace_root: &Path) -> Option<String> {
    git_output(workspace_root, &["branch", "--show-current"])
        .filter(|branch| !branch.is_empty())
        .or_else(|| git_output(workspace_root, &["rev-parse", "--short", "HEAD"]))
}

fn git_output(workspace_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_context_renders_requested_fields() {
        let rendered = render_environment_context(&EnvironmentContext {
            cwd: "/repo".to_string(),
            shell: "zsh".to_string(),
            current_date: "2026-05-20".to_string(),
            timezone: "America/Chicago".to_string(),
            git_repo: Some("https://github.com/everruns/everruns.git".to_string()),
            git_user: Some("Chal & Yi".to_string()),
            git_email: Some("chalyi@example.com".to_string()),
            git_current_branch: Some("feature<context>".to_string()),
        });

        assert!(rendered.starts_with("<environment_context>\n"));
        assert!(rendered.contains("  <cwd>/repo</cwd>\n"));
        assert!(rendered.contains("  <shell>zsh</shell>\n"));
        assert!(rendered.contains("  <current_date>2026-05-20</current_date>\n"));
        assert!(rendered.contains("  <timezone>America/Chicago</timezone>\n"));
        assert!(
            rendered.contains("  <git_repo>https://github.com/everruns/everruns.git</git_repo>\n")
        );
        assert!(rendered.contains("  <git_user>Chal &amp; Yi</git_user>\n"));
        assert!(rendered.contains("  <git_email>chalyi@example.com</git_email>\n"));
        assert!(
            rendered
                .contains("  <git_current_branch>feature&lt;context&gt;</git_current_branch>\n")
        );
        assert!(rendered.ends_with("</environment_context>"));
    }
}
