// Deterministic progress-reporting helpers for external handoff channels.
//
// Design Decision: The agent-facing tool is generic (`report_progress`) while
// delivery remains channel-specific. Handoff sessions opt in via a session tag
// (`{platform}:reply_mode:report_progress_only`), which lets ReasonAtom expose
// the tool and prompt without leaking platform-specific behavior into the wider
// runtime.
//
// Design Decision: Tags use a generic `channel:reply_mode:*` prefix that all
// platforms share, plus legacy `slack:reply_mode:*` aliases for backward compat.
// `session_uses_report_progress()` checks both prefixes.

use crate::RuntimeAgent;
use crate::app::SlackReplyMode;
use crate::channel::ChannelReplyMode;
use crate::tool_types::ToolDefinition;
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const REPORT_PROGRESS_TOOL_NAME: &str = "report_progress";

// Generic channel-agnostic tag constants
pub const CHANNEL_REPLY_MODE_TAG_PREFIX: &str = "channel:reply_mode:";
pub const CHANNEL_REPORT_PROGRESS_ONLY_TAG: &str = "channel:reply_mode:report_progress_only";

// Legacy Slack-specific tag constants (kept for backward compat)
pub const SLACK_REPLY_MODE_TAG_PREFIX: &str = "slack:reply_mode:";
pub const SLACK_REPORT_PROGRESS_ONLY_TAG: &str = "slack:reply_mode:report_progress_only";
const REPORT_PROGRESS_PROMPT_MARKER: &str = "# External Progress Reporting";
const REPORT_PROGRESS_SYSTEM_PROMPT: &str = r#"# External Progress Reporting

This session is attached to an external handoff thread.

The external user does not see normal assistant messages. They only see updates sent through `report_progress`.

Rules:
- Use `report_progress` for meaningful user-facing updates only.
- Use status `progress` for material milestones, `blocked` when waiting or stuck, and `completed` before the turn ends.
- Keep summaries concise, deterministic, and focused on outcomes.
- Do not mirror low-level tool chatter into `report_progress`."#;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressReportStatus {
    Progress,
    Blocked,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressReportPayload {
    pub status: ProgressReportStatus,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

impl ProgressReportPayload {
    fn validate(self) -> Result<Self, String> {
        let summary = self.summary.trim();
        if summary.is_empty() {
            return Err("Missing required field: 'summary'".to_string());
        }

        let mut details = Vec::with_capacity(self.details.len());
        for (idx, detail) in self.details.iter().enumerate() {
            let trimmed = detail.trim();
            if trimmed.is_empty() {
                return Err(format!("details[{}] must not be empty", idx));
            }
            details.push(trimmed.to_string());
        }

        Ok(Self {
            status: self.status,
            summary: summary.to_string(),
            details,
        })
    }
}

/// Check if a session uses report_progress mode.
/// Checks both the generic `channel:reply_mode:*` and legacy `slack:reply_mode:*` tags.
pub fn session_uses_report_progress(tags: &[String]) -> bool {
    tags.iter()
        .any(|tag| tag == CHANNEL_REPORT_PROGRESS_ONLY_TAG || tag == SLACK_REPORT_PROGRESS_ONLY_TAG)
}

/// Sync channel-agnostic reply mode tags on a session.
/// Sets the generic `channel:reply_mode:*` tag used by all platforms.
pub fn sync_channel_reply_mode_tags(tags: &mut Vec<String>, reply_mode: ChannelReplyMode) {
    tags.retain(|tag| !tag.starts_with(CHANNEL_REPLY_MODE_TAG_PREFIX));
    if reply_mode == ChannelReplyMode::ReportProgressOnly {
        tags.push(CHANNEL_REPORT_PROGRESS_ONLY_TAG.to_string());
    }
}

/// Sync Slack-specific reply mode tags (legacy — delegates to channel-agnostic version
/// and also sets the `slack:reply_mode:*` tag for backward compat with existing sessions).
pub fn sync_slack_reply_mode_tags(tags: &mut Vec<String>, reply_mode: SlackReplyMode) {
    tags.retain(|tag| !tag.starts_with(SLACK_REPLY_MODE_TAG_PREFIX));
    if reply_mode == SlackReplyMode::ReportProgressOnly {
        tags.push(SLACK_REPORT_PROGRESS_ONLY_TAG.to_string());
    }
    // Also set the generic tag so new code paths work
    sync_channel_reply_mode_tags(tags, reply_mode.into());
}

pub fn report_progress_tool_definition() -> ToolDefinition {
    ReportProgressTool.to_definition()
}

pub fn apply_report_progress_mode(mut runtime_agent: RuntimeAgent) -> RuntimeAgent {
    if !runtime_agent
        .tools
        .iter()
        .any(|tool| tool.name() == REPORT_PROGRESS_TOOL_NAME)
    {
        runtime_agent.tools.push(report_progress_tool_definition());
    }

    if !runtime_agent
        .system_prompt
        .contains(REPORT_PROGRESS_PROMPT_MARKER)
    {
        runtime_agent.system_prompt = if runtime_agent.system_prompt.is_empty() {
            REPORT_PROGRESS_SYSTEM_PROMPT.to_string()
        } else {
            format!(
                "{}\n\n{}",
                REPORT_PROGRESS_SYSTEM_PROMPT, runtime_agent.system_prompt
            )
        };
    }

    runtime_agent
}

/// Format a progress report as plain text (platform-agnostic default).
/// Platform adapters can override via ChannelDeliveryAdapter::format_progress_report().
pub fn format_progress_report(report: &ProgressReportPayload) -> String {
    let heading = match report.status {
        ProgressReportStatus::Progress => "Update",
        ProgressReportStatus::Blocked => "Blocked",
        ProgressReportStatus::Completed => "Done",
    };

    let mut lines = vec![format!("{}: {}", heading, report.summary)];
    for detail in &report.details {
        lines.push(format!("- {}", detail));
    }
    lines.join("\n")
}

/// Format a progress report for Slack (uses same format today, kept for compat).
pub fn format_progress_report_for_slack(report: &ProgressReportPayload) -> String {
    let heading = match report.status {
        ProgressReportStatus::Progress => "Update",
        ProgressReportStatus::Blocked => "Blocked",
        ProgressReportStatus::Completed => "Done",
    };

    let mut lines = vec![format!("{}: {}", heading, report.summary)];
    for detail in &report.details {
        lines.push(format!("- {}", detail));
    }
    lines.join("\n")
}

pub struct ReportProgressTool;

#[async_trait]
impl Tool for ReportProgressTool {
    fn name(&self) -> &str {
        REPORT_PROGRESS_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Report Progress")
    }

    fn description(&self) -> &str {
        "Send a deterministic, user-facing progress update for an external handoff thread. Use status 'completed' before ending the turn."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["progress", "blocked", "completed"],
                    "description": "Kind of progress update being reported."
                },
                "summary": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Short user-facing summary of the current milestone or outcome."
                },
                "details": {
                    "type": "array",
                    "description": "Optional short bullet points with concrete outcomes or blockers.",
                    "items": {
                        "type": "string",
                        "minLength": 1
                    }
                }
            },
            "required": ["status", "summary"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let payload = match serde_json::from_value::<ProgressReportPayload>(arguments) {
            Ok(payload) => payload,
            Err(error) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid report_progress arguments: {}",
                    error
                ));
            }
        };

        match payload.validate() {
            Ok(validated) => ToolExecutionResult::success(serde_json::to_value(validated).unwrap()),
            Err(error) => ToolExecutionResult::tool_error(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_report_progress_tool_success() {
        let tool = ReportProgressTool;
        let result = tool
            .execute(serde_json::json!({
                "status": "completed",
                "summary": "Fixed the failing Slack session routing",
                "details": ["added reply-mode tags", "wired deterministic progress delivery"]
            }))
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                let payload: ProgressReportPayload = serde_json::from_value(value).unwrap();
                assert_eq!(payload.status, ProgressReportStatus::Completed);
                assert_eq!(payload.summary, "Fixed the failing Slack session routing");
                assert_eq!(payload.details.len(), 2);
            }
            other => panic!("Expected success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_report_progress_tool_rejects_empty_summary() {
        let tool = ReportProgressTool;
        let result = tool
            .execute(serde_json::json!({
                "status": "progress",
                "summary": "   "
            }))
            .await;

        match result {
            ToolExecutionResult::ToolError(message) => {
                assert_eq!(message, "Missing required field: 'summary'");
            }
            other => panic!("Expected tool error, got {:?}", other),
        }
    }

    #[test]
    fn test_sync_slack_reply_mode_tags() {
        let mut tags = vec![
            "slack:app:app_123".to_string(),
            "slack:thread:123.456".to_string(),
            "slack:reply_mode:all_messages".to_string(),
        ];

        sync_slack_reply_mode_tags(&mut tags, SlackReplyMode::ReportProgressOnly);

        // Should have both legacy slack: and generic channel: tags
        assert!(tags.iter().any(|tag| tag == SLACK_REPORT_PROGRESS_ONLY_TAG));
        assert!(
            tags.iter()
                .any(|tag| tag == CHANNEL_REPORT_PROGRESS_ONLY_TAG)
        );
        assert_eq!(
            tags.iter()
                .filter(|tag| tag.starts_with(SLACK_REPLY_MODE_TAG_PREFIX))
                .count(),
            1
        );
    }

    #[test]
    fn test_sync_channel_reply_mode_tags() {
        let mut tags = vec!["other:tag".to_string()];

        sync_channel_reply_mode_tags(&mut tags, ChannelReplyMode::ReportProgressOnly);
        assert!(
            tags.iter()
                .any(|tag| tag == CHANNEL_REPORT_PROGRESS_ONLY_TAG)
        );

        sync_channel_reply_mode_tags(&mut tags, ChannelReplyMode::AllMessages);
        assert!(
            !tags
                .iter()
                .any(|tag| tag.starts_with(CHANNEL_REPLY_MODE_TAG_PREFIX))
        );
    }

    #[test]
    fn test_session_uses_report_progress_generic_tag() {
        let tags = vec![CHANNEL_REPORT_PROGRESS_ONLY_TAG.to_string()];
        assert!(session_uses_report_progress(&tags));
    }

    #[test]
    fn test_session_uses_report_progress_legacy_tag() {
        let tags = vec![SLACK_REPORT_PROGRESS_ONLY_TAG.to_string()];
        assert!(session_uses_report_progress(&tags));
    }

    #[test]
    fn test_apply_report_progress_mode_adds_tool_and_prompt_once() {
        let runtime_agent = RuntimeAgent::new("Base prompt.", "gpt-5.4");

        let runtime_agent = apply_report_progress_mode(runtime_agent);
        let runtime_agent = apply_report_progress_mode(runtime_agent);

        assert_eq!(
            runtime_agent
                .tools
                .iter()
                .filter(|tool| tool.name() == REPORT_PROGRESS_TOOL_NAME)
                .count(),
            1
        );
        assert_eq!(
            runtime_agent
                .system_prompt
                .matches(REPORT_PROGRESS_PROMPT_MARKER)
                .count(),
            1
        );
    }

    #[test]
    fn test_format_progress_report_for_slack() {
        let text = format_progress_report_for_slack(&ProgressReportPayload {
            status: ProgressReportStatus::Blocked,
            summary: "Waiting on credentials".to_string(),
            details: vec!["need Slack bot token".to_string()],
        });

        assert_eq!(
            text,
            "Blocked: Waiting on credentials\n- need Slack bot token"
        );
    }
}
