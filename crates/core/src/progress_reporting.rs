// Deterministic progress-reporting helpers for external handoff channels.
//
// Design Decision: The agent-facing tool is generic (`report_progress`) while
// delivery remains channel-specific. Handoff sessions opt in via session tags
// (`channel:reply_mode:report_progress_only` generically, or
// `slack:reply_mode:report_progress_only` for legacy Slack), which lets ReasonAtom expose
// the tool and prompt without leaking platform-specific behavior into the wider
// runtime.
//
// Design Decision: Tags use a generic `channel:reply_mode:*` prefix that all
// platforms share, plus legacy `slack:reply_mode:*` aliases for backward compat.
// `session_uses_report_progress()` checks both prefixes.

use crate::RuntimeAgent;
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
///
/// Takes the neutral [`ChannelReplyMode`]; callers holding the Slack-specific
/// `SlackReplyMode` (now owned by `everruns-platform`) convert with `.into()`.
pub fn sync_slack_reply_mode_tags(tags: &mut Vec<String>, reply_mode: ChannelReplyMode) {
    tags.retain(|tag| !tag.starts_with(SLACK_REPLY_MODE_TAG_PREFIX));
    if reply_mode == ChannelReplyMode::ReportProgressOnly {
        tags.push(SLACK_REPORT_PROGRESS_ONLY_TAG.to_string());
    }
    // Also set the generic tag so new code paths work
    sync_channel_reply_mode_tags(tags, reply_mode);
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
        for status in ["progress", "blocked", "completed"] {
            let result=ReportProgressTool.execute(serde_json::json!({"status":status,"summary":" \tSummary α\n","details":[" first ","\nsecond\t"]})).await;
            let ToolExecutionResult::Success(value) = result else {
                panic!("expected success: {result:?}")
            };
            assert_eq!(
                value,
                serde_json::json!({"status":status,"summary":"Summary α","details":["first","second"]})
            );
        }
        let result = ReportProgressTool
            .execute(serde_json::json!({"status":"progress","summary":"No details"}))
            .await;
        assert!(
            matches!(result,ToolExecutionResult::Success(value) if value==serde_json::json!({"status":"progress","summary":"No details"}))
        );
    }

    #[tokio::test]
    async fn report_progress_rejects_invalid_arguments_with_useful_errors() {
        for args in [
            serde_json::json!({}),
            serde_json::json!({"status":"unknown","summary":"ok"}),
            serde_json::json!({"status":"progress","summary":42}),
            serde_json::json!({"status":"progress","summary":"ok","details":[42]}),
        ] {
            assert!(
                matches!(ReportProgressTool.execute(args).await,ToolExecutionResult::ToolError(message) if message.starts_with("Invalid report_progress arguments:"))
            );
        }
        for (args, error) in [
            (
                serde_json::json!({"status":"progress","summary":" \t\n"}),
                "Missing required field: 'summary'",
            ),
            (
                serde_json::json!({"status":"progress","summary":"ok","details":[" "]}),
                "details[0] must not be empty",
            ),
            (
                serde_json::json!({"status":"progress","summary":"ok","details":["valid","\u{2003}"]}),
                "details[1] must not be empty",
            ),
        ] {
            assert!(
                matches!(ReportProgressTool.execute(args).await,ToolExecutionResult::ToolError(message) if message==error)
            );
        }
    }

    #[test]
    fn test_sync_slack_reply_mode_tags() {
        let mut tags = vec![
            "slack:app:app_123".into(),
            "slack:thread:123.456".into(),
            "slack:reply_mode:all_messages".into(),
            "slack:reply_mode:report_progress_only".into(),
            "channel:reply_mode:all_messages".into(),
            "other:tag".into(),
        ];
        let unrelated = vec!["slack:app:app_123", "slack:thread:123.456", "other:tag"];
        sync_slack_reply_mode_tags(&mut tags, ChannelReplyMode::ReportProgressOnly);
        assert_eq!(
            tags,
            [
                "slack:app:app_123",
                "slack:thread:123.456",
                "other:tag",
                "slack:reply_mode:report_progress_only",
                "channel:reply_mode:report_progress_only"
            ]
        );
        let once = tags.clone();
        sync_slack_reply_mode_tags(&mut tags, ChannelReplyMode::ReportProgressOnly);
        assert_eq!(tags, once);
        sync_slack_reply_mode_tags(&mut tags, ChannelReplyMode::AllMessages);
        assert_eq!(tags, unrelated);
        assert!(!session_uses_report_progress(&tags));
    }

    #[test]
    fn test_sync_channel_reply_mode_tags() {
        let mut tags = vec![
            "other:tag".into(),
            "channel:reply_mode:obsolete".into(),
            "channel:reply_mode:report_progress_only".into(),
            "channel:reply_modes:decoy".into(),
        ];
        sync_channel_reply_mode_tags(&mut tags, ChannelReplyMode::ReportProgressOnly);
        assert_eq!(
            tags,
            [
                "other:tag",
                "channel:reply_modes:decoy",
                "channel:reply_mode:report_progress_only"
            ]
        );
        let once = tags.clone();
        sync_channel_reply_mode_tags(&mut tags, ChannelReplyMode::ReportProgressOnly);
        assert_eq!(tags, once);
        sync_channel_reply_mode_tags(&mut tags, ChannelReplyMode::AllMessages);
        assert_eq!(tags, ["other:tag", "channel:reply_modes:decoy"]);
    }

    #[test]
    fn progress_mode_requires_exact_generic_or_legacy_tag() {
        for (tags, expected) in [
            (vec![], false),
            (vec!["channel:reply_mode:report_progress_only"], true),
            (vec!["slack:reply_mode:report_progress_only"], true),
            (
                vec!["other", "channel:reply_mode:report_progress_only"],
                true,
            ),
            (
                vec![
                    "channel:reply_mode:report_progress_only_extra",
                    "slack:reply_mode:all_messages",
                    "channel:reply_mode:",
                ],
                false,
            ),
        ] {
            assert_eq!(
                session_uses_report_progress(
                    &tags.into_iter().map(str::to_string).collect::<Vec<_>>()
                ),
                expected
            );
        }
    }

    #[test]
    fn test_apply_report_progress_mode_adds_tool_and_prompt_once() {
        for base in ["", "Base prompt."] {
            let mut agent = RuntimeAgent::new(base, "model-42");
            agent.max_tokens = Some(17);
            agent.parallel_tool_calls = Some(false);
            let first = apply_report_progress_mode(agent);
            assert_eq!(first.model, "model-42");
            assert_eq!(first.max_tokens, Some(17));
            assert_eq!(first.parallel_tool_calls, Some(false));
            assert_eq!(
                first.tools.iter().map(|t| t.name()).collect::<Vec<_>>(),
                ["report_progress"]
            );
            assert!(
                first
                    .system_prompt
                    .starts_with("# External Progress Reporting\n")
            );
            assert!(first.system_prompt.contains("before the turn ends"));
            if !base.is_empty() {
                assert!(first.system_prompt.ends_with("\n\nBase prompt."));
            }
            let before = serde_json::to_value(&first).unwrap();
            assert_eq!(
                serde_json::to_value(apply_report_progress_mode(first)).unwrap(),
                before
            );
        }
        let mut agent = RuntimeAgent::new("Base", "model-42");
        let mut tool = report_progress_tool_definition();
        if let ToolDefinition::Builtin(ref mut definition) = tool {
            definition.description = "Custom existing description".into();
        }
        agent.tools.push(tool.clone());
        let result = apply_report_progress_mode(agent);
        assert_eq!(
            serde_json::to_value(result.tools).unwrap(),
            serde_json::to_value(vec![tool]).unwrap()
        );
    }

    #[test]
    fn both_progress_formatters_preserve_status_headings_and_detail_order() {
        for (status, heading) in [
            (ProgressReportStatus::Progress, "Update"),
            (ProgressReportStatus::Blocked, "Blocked"),
            (ProgressReportStatus::Completed, "Done"),
        ] {
            for details in [vec![], vec!["First α".into(), "Second".into()]] {
                let report = ProgressReportPayload {
                    status,
                    summary: "Summary".into(),
                    details,
                };
                let expected = if report.details.is_empty() {
                    format!("{heading}: Summary")
                } else {
                    format!("{heading}: Summary\n- First α\n- Second")
                };
                assert_eq!(format_progress_report(&report), expected);
                assert_eq!(format_progress_report_for_slack(&report), expected);
            }
        }
    }
}
