//! Native Slack capability: act as the channel's own bot (EVE-1024).
//!
//! The first channel adapter to implement `Capability::tools()`. An agent
//! deployed to Slack could previously only reply in its own thread; reaching
//! anything else in the Slack API meant attaching a Slack MCP server, which
//! works but costs a *second* bot token — separate scopes to grant, separate
//! rotation, and an identity that is not the bot the user invited.
//!
//! These tools act as the channel's own bot instead. They carry no credential:
//! each one names an action and hands it to the [`SlackActionInvoker`] seam,
//! which the control plane implements. See
//! [`crate::slack_action`] for why the action travels and the token does not,
//! and `knowledge/integrations/slack-agent-actions.md` for the design.
//!
//! The Slack MCP server stays supported for anything exotic. This removes the
//! second credential for the common cases; it does not replace MCP.

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};

use crate::slack_action::{
    SlackAction, SlackActionError, SlackActionInvokerExt, SlackActionOutcome,
};

pub const SLACK_CAPABILITY_ID: &str = "slack";

/// Largest file an agent may share, before base64 or multipart overhead.
///
/// THREAT[TM-DOS-031]: `upload_file` is the only tool here whose argument size
/// is model-chosen and unbounded by Slack's own limits at the point we accept
/// it. The cap is applied before the bytes cross the invoker seam so an
/// oversized argument costs a rejected tool call rather than a transfer.
const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;

/// Slack capability: the agent acts as the bot already in the room.
pub struct SlackCapability;

impl Capability for SlackCapability {
    fn id(&self) -> &str {
        SLACK_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Slack"
    }

    fn description(&self) -> &str {
        "Act in the Slack conversation as the bot the workspace already invited: react to \
         messages, update them, share files, and look users up."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Slack",
            "Дії у розмові Slack від імені бота, якого вже додано в робочий простір: реакції, \
             оновлення повідомлень, надсилання файлів і пошук користувачів.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("slack")
    }

    fn category(&self) -> Option<&str> {
        Some("Integrations")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        // States the one precondition the model cannot discover by itself, and
        // the channel/timestamp convention, so a first call does not have to
        // fail to teach it. Deliberately does not list the tools: their own
        // descriptions do that.
        Some(
            "These tools act as this workspace's Slack bot and only work in a session a Slack \
             message created. `channel` and `timestamp` come from the Slack message you are \
             replying to; `timestamp` is Slack's `ts` value, not a date. Prefer a reaction over a \
             message when you only need to acknowledge something.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        // `post_to_channel` is deliberately absent. It widens blast radius from
        // "the thread that asked" to "anywhere the bot is", and nothing needs
        // it yet: the reply path already answers in the thread. It returns when
        // there is a per-channel allowlist to gate it (EVE-1024).
        vec![
            Box::new(SlackAddReactionTool),
            Box::new(SlackUpdateMessageTool),
            Box::new(SlackLookupUserTool),
            Box::new(SlackUploadFileTool),
        ]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["slack_actions"]
    }
}

// ============================================================================
// Shared execution path
// ============================================================================

fn action_error_to_result(err: SlackActionError) -> ToolExecutionResult {
    if err.is_tool_error() {
        ToolExecutionResult::tool_error(err.to_string())
    } else {
        ToolExecutionResult::internal_error_msg(err.to_string())
    }
}

/// Resolve the invoker and run `action`, shaping the outcome into a tool result.
///
/// Every tool in this capability funnels through here so the fail-closed
/// behaviour — no invoker installed, or no Slack session — is decided once
/// rather than per tool.
async fn run(context: &ToolContext, action: SlackAction) -> ToolExecutionResult {
    let Some(invoker) = context.extensions.get::<SlackActionInvokerExt>() else {
        // The capability is enabled on a deployment that does not install the
        // seam — a remote worker on a control plane that predates the RPC, or a
        // Framework registry. Same answer as a non-Slack session from the
        // model's side: the tool cannot apply here.
        return ToolExecutionResult::tool_error(SlackActionError::NoSlackSession.to_string());
    };

    match invoker.0.invoke(action).await {
        Ok(outcome) => ToolExecutionResult::success(outcome_to_json(outcome)),
        Err(err) => action_error_to_result(err),
    }
}

fn outcome_to_json(outcome: SlackActionOutcome) -> Value {
    match outcome {
        SlackActionOutcome::ReactionAdded { already_reacted } => json!({
            "success": true,
            "already_reacted": already_reacted,
        }),
        SlackActionOutcome::MessageUpdated { channel, timestamp } => json!({
            "success": true,
            "channel": channel,
            "timestamp": timestamp,
        }),
        SlackActionOutcome::User {
            user_id,
            display_name,
            real_name,
            is_bot,
            tz,
        } => json!({
            "user_id": user_id,
            "display_name": display_name,
            "real_name": real_name,
            "is_bot": is_bot,
            "tz": tz,
        }),
        SlackActionOutcome::FileUploaded { file_id, permalink } => json!({
            "success": true,
            "file_id": file_id,
            "permalink": permalink,
        }),
    }
}

/// Read a required string argument, rejecting an empty one.
///
/// Slack answers a blank `channel` or `ts` with a generic `invalid_arguments`,
/// so checking here turns a round trip into an immediate, specific message.
fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, ToolExecutionResult> {
    match arguments.get(key).and_then(|v| v.as_str()) {
        Some(value) if !value.trim().is_empty() => Ok(value),
        Some(_) => Err(ToolExecutionResult::tool_error(format!(
            "Parameter `{key}` must not be empty"
        ))),
        None => Err(ToolExecutionResult::tool_error(format!(
            "Missing required parameter: {key}"
        ))),
    }
}

fn optional_str(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

const CHANNEL_SCHEMA_DESCRIPTION: &str = "Slack channel ID the message is in (e.g. \"C0123456789\"), as it appeared on the message \
     you are replying to.";
const TIMESTAMP_SCHEMA_DESCRIPTION: &str =
    "The Slack message's `ts` value (e.g. \"1728394857.123456\"). Not a date.";

// ============================================================================
// slack_add_reaction
// ============================================================================

pub struct SlackAddReactionTool;

#[async_trait]
impl Tool for SlackAddReactionTool {
    fn name(&self) -> &str {
        "slack_add_reaction"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Slack: Add Reaction")
    }

    fn description(&self) -> &str {
        "Add an emoji reaction to a Slack message as this workspace's bot. The cheapest way to \
         acknowledge a request without posting a message."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "description": CHANNEL_SCHEMA_DESCRIPTION },
                "timestamp": { "type": "string", "description": TIMESTAMP_SCHEMA_DESCRIPTION },
                "name": {
                    "type": "string",
                    "description": "Emoji name without colons (e.g. \"eyes\", \"white_check_mark\")."
                }
            },
            "required": ["channel", "timestamp", "name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "slack_add_reaction requires session context. This tool must be executed with \
             session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let channel = match required_str(&arguments, "channel") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        let timestamp = match required_str(&arguments, "timestamp") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        let name = match required_str(&arguments, "name") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        // Models reach for `:eyes:` because that is how a reaction is written in
        // Slack's own message syntax, but `reactions.add` wants the bare name
        // and answers the colons with `invalid_name`.
        let name = name.trim_matches(':').to_string();
        if name.is_empty() {
            return ToolExecutionResult::tool_error(
                "Parameter `name` must be an emoji name, not just colons",
            );
        }

        run(
            context,
            SlackAction::AddReaction {
                channel,
                timestamp,
                name,
            },
        )
        .await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// slack_update_message
// ============================================================================

pub struct SlackUpdateMessageTool;

#[async_trait]
impl Tool for SlackUpdateMessageTool {
    fn name(&self) -> &str {
        "slack_update_message"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Slack: Update Message")
    }

    fn description(&self) -> &str {
        "Rewrite a Slack message this bot posted. Use it to turn a status message into its \
         result instead of posting a second message."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "description": CHANNEL_SCHEMA_DESCRIPTION },
                "timestamp": {
                    "type": "string",
                    "description": "The `ts` of the bot message to rewrite. Only messages this \
                                    bot posted can be updated."
                },
                "text": {
                    "type": "string",
                    "description": "Replacement message text. Markdown is rendered."
                }
            },
            "required": ["channel", "timestamp", "text"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "slack_update_message requires session context. This tool must be executed with \
             session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let channel = match required_str(&arguments, "channel") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        let timestamp = match required_str(&arguments, "timestamp") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        // `chat.update` treats an empty `text` with no blocks as a delete
        // request and answers `no_text`. Reject it here so an empty-string
        // argument does not read as "clear the message".
        let text = match required_str(&arguments, "text") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };

        run(
            context,
            SlackAction::UpdateMessage {
                channel,
                timestamp,
                text,
            },
        )
        .await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// slack_lookup_user
// ============================================================================

pub struct SlackLookupUserTool;

#[async_trait]
impl Tool for SlackLookupUserTool {
    fn name(&self) -> &str {
        "slack_lookup_user"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Slack: Look Up User")
    }

    fn description(&self) -> &str {
        "Resolve a Slack user ID to that person's display name, real name, timezone, and whether \
         they are a bot. Use it to address someone by name rather than by ID."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Slack user ID (e.g. \"U0123456789\"), as it appeared on a \
                                    message or mention."
                }
            },
            "required": ["user_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "slack_lookup_user requires session context. This tool must be executed with session \
             context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let user_id = match required_str(&arguments, "user_id") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        // Mentions arrive in message text as `<@U123>`, and that is the form a
        // model copies. Accept it rather than making the round trip fail.
        let user_id = user_id
            .trim_start_matches("<@")
            .trim_end_matches('>')
            .to_string();
        if user_id.is_empty() {
            return ToolExecutionResult::tool_error("Parameter `user_id` must be a Slack user ID");
        }

        run(context, SlackAction::LookupUser { user_id }).await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// slack_upload_file
// ============================================================================

pub struct SlackUploadFileTool;

#[async_trait]
impl Tool for SlackUploadFileTool {
    fn name(&self) -> &str {
        "slack_upload_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Slack: Upload File")
    }

    fn description(&self) -> &str {
        "Share a file into the Slack conversation as this workspace's bot. Pass the file's text \
         content; use it for reports, diffs, and logs that are too long to read in a message."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "string",
                    "description": "Slack channel ID to share the file into."
                },
                "thread_ts": {
                    "type": "string",
                    "description": "Thread `ts` to share into. Omit to post at channel level."
                },
                "filename": {
                    "type": "string",
                    "description": "Filename shown in Slack, including its extension \
                                    (e.g. \"report.md\")."
                },
                "content": {
                    "type": "string",
                    "description": "The file's text content."
                },
                "initial_comment": {
                    "type": "string",
                    "description": "Optional message posted alongside the file."
                }
            },
            "required": ["channel", "filename", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "slack_upload_file requires session context. This tool must be executed with session \
             context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let channel = match required_str(&arguments, "channel") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        let filename = match required_str(&arguments, "filename") {
            Ok(value) => value.to_string(),
            Err(result) => return result,
        };
        // A zero-byte upload is rejected by `files.getUploadURLExternal`, so an
        // empty `content` is caught here rather than after two round trips.
        let content = match required_str(&arguments, "content") {
            Ok(value) => value.as_bytes().to_vec(),
            Err(result) => return result,
        };
        if content.len() > MAX_UPLOAD_BYTES {
            return ToolExecutionResult::tool_error(format!(
                "File content is {} bytes, over the {} byte limit for a Slack upload",
                content.len(),
                MAX_UPLOAD_BYTES
            ));
        }

        run(
            context,
            SlackAction::UploadFile {
                channel,
                thread_ts: optional_str(&arguments, "thread_ts"),
                filename,
                content,
                initial_comment: optional_str(&arguments, "initial_comment"),
            },
        )
        .await
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::typed_id::SessionId;
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Records what reached the seam so a test can assert on normalization.
    struct RecordingInvoker {
        seen: Mutex<Vec<SlackAction>>,
        outcome: SlackActionOutcome,
    }

    impl RecordingInvoker {
        fn new(outcome: SlackActionOutcome) -> Self {
            Self {
                seen: Mutex::new(Vec::new()),
                outcome,
            }
        }
    }

    #[async_trait]
    impl crate::slack_action::SlackActionInvoker for RecordingInvoker {
        async fn invoke(
            &self,
            action: SlackAction,
        ) -> Result<SlackActionOutcome, SlackActionError> {
            self.seen.lock().expect("poisoned").push(action);
            Ok(self.outcome.clone())
        }
    }

    /// An invoker that fails the way the control plane does for a session with
    /// no `slack:endpoint:` tag.
    struct NoSlackSessionInvoker;

    #[async_trait]
    impl crate::slack_action::SlackActionInvoker for NoSlackSessionInvoker {
        async fn invoke(
            &self,
            _action: SlackAction,
        ) -> Result<SlackActionOutcome, SlackActionError> {
            Err(SlackActionError::NoSlackSession)
        }
    }

    fn context_with(invoker: Arc<dyn crate::slack_action::SlackActionInvoker>) -> ToolContext {
        let mut context = ToolContext::new(SessionId::new());
        context
            .extensions
            .insert(Arc::new(SlackActionInvokerExt(invoker)));
        context
    }

    /// A context with no invoker installed at all, as a deployment that does
    /// not wire the seam produces.
    fn context_without_invoker() -> ToolContext {
        ToolContext::new(SessionId::new())
    }

    #[tokio::test]
    async fn add_reaction_forwards_the_action() {
        let invoker = Arc::new(RecordingInvoker::new(SlackActionOutcome::ReactionAdded {
            already_reacted: false,
        }));
        let context = context_with(invoker.clone());

        let result = SlackAddReactionTool
            .execute_with_context(
                json!({ "channel": "C1", "timestamp": "1.2", "name": "eyes" }),
                &context,
            )
            .await;

        assert!(result.is_success(), "unexpected failure: {result:?}");
        let seen = invoker.seen.lock().expect("poisoned");
        assert_eq!(
            seen.as_slice(),
            &[SlackAction::AddReaction {
                channel: "C1".to_string(),
                timestamp: "1.2".to_string(),
                name: "eyes".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn add_reaction_strips_the_colons_a_model_copies_from_slack_syntax() {
        let invoker = Arc::new(RecordingInvoker::new(SlackActionOutcome::ReactionAdded {
            already_reacted: false,
        }));
        let context = context_with(invoker.clone());

        let result = SlackAddReactionTool
            .execute_with_context(
                json!({ "channel": "C1", "timestamp": "1.2", "name": ":eyes:" }),
                &context,
            )
            .await;

        assert!(result.is_success(), "unexpected failure: {result:?}");
        let seen = invoker.seen.lock().expect("poisoned");
        assert!(matches!(
            &seen[0],
            SlackAction::AddReaction { name, .. } if name == "eyes"
        ));
    }

    #[tokio::test]
    async fn add_reaction_rejects_colons_with_no_name() {
        let invoker = Arc::new(RecordingInvoker::new(SlackActionOutcome::ReactionAdded {
            already_reacted: false,
        }));
        let context = context_with(invoker.clone());

        let result = SlackAddReactionTool
            .execute_with_context(
                json!({ "channel": "C1", "timestamp": "1.2", "name": "::" }),
                &context,
            )
            .await;

        assert!(!result.is_success());
        assert!(
            invoker.seen.lock().expect("poisoned").is_empty(),
            "a rejected argument must not reach the seam"
        );
    }

    #[tokio::test]
    async fn lookup_user_accepts_the_mention_form() {
        let invoker = Arc::new(RecordingInvoker::new(SlackActionOutcome::User {
            user_id: "U1".to_string(),
            display_name: Some("ada".to_string()),
            real_name: None,
            is_bot: false,
            tz: None,
        }));
        let context = context_with(invoker.clone());

        let result = SlackLookupUserTool
            .execute_with_context(json!({ "user_id": "<@U1>" }), &context)
            .await;

        assert!(result.is_success(), "unexpected failure: {result:?}");
        let seen = invoker.seen.lock().expect("poisoned");
        assert_eq!(
            seen.as_slice(),
            &[SlackAction::LookupUser {
                user_id: "U1".to_string()
            }]
        );
    }

    #[tokio::test]
    async fn upload_file_rejects_content_over_the_cap() {
        let invoker = Arc::new(RecordingInvoker::new(SlackActionOutcome::FileUploaded {
            file_id: "F1".to_string(),
            permalink: None,
        }));
        let context = context_with(invoker.clone());

        let oversized = "x".repeat(MAX_UPLOAD_BYTES + 1);
        let result = SlackUploadFileTool
            .execute_with_context(
                json!({ "channel": "C1", "filename": "big.txt", "content": oversized }),
                &context,
            )
            .await;

        assert!(!result.is_success());
        assert!(
            invoker.seen.lock().expect("poisoned").is_empty(),
            "an oversized argument must be rejected before it crosses the seam"
        );
    }

    #[tokio::test]
    async fn upload_file_omits_absent_optional_arguments() {
        let invoker = Arc::new(RecordingInvoker::new(SlackActionOutcome::FileUploaded {
            file_id: "F1".to_string(),
            permalink: None,
        }));
        let context = context_with(invoker.clone());

        let result = SlackUploadFileTool
            .execute_with_context(
                json!({
                    "channel": "C1",
                    "filename": "report.md",
                    "content": "hello",
                    "thread_ts": "   "
                }),
                &context,
            )
            .await;

        assert!(result.is_success(), "unexpected failure: {result:?}");
        let seen = invoker.seen.lock().expect("poisoned");
        assert!(matches!(
            &seen[0],
            SlackAction::UploadFile {
                thread_ts: None,
                initial_comment: None,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn tools_fail_closed_without_a_slack_session() {
        let context = context_with(Arc::new(NoSlackSessionInvoker));

        let result = SlackAddReactionTool
            .execute_with_context(
                json!({ "channel": "C1", "timestamp": "1.2", "name": "eyes" }),
                &context,
            )
            .await;

        assert!(!result.is_success());
        let rendered = format!("{result:?}");
        assert!(
            rendered.contains("did not originate from Slack"),
            "the reason must name the precondition: {rendered}"
        );
    }

    #[tokio::test]
    async fn tools_fail_closed_when_the_seam_is_not_installed() {
        let context = context_without_invoker();

        let result = SlackLookupUserTool
            .execute_with_context(json!({ "user_id": "U1" }), &context)
            .await;

        assert!(!result.is_success());
    }

    #[test]
    fn post_to_channel_is_not_offered() {
        // Guards the scope decision in `tools()`: a wider-blast-radius post
        // must not appear without the per-channel allowlist that gates it.
        let names: Vec<String> = SlackCapability
            .tools()
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "slack_add_reaction",
                "slack_update_message",
                "slack_lookup_user",
                "slack_upload_file",
            ]
        );
    }
}
