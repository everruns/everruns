// BTW capability
//
// Decisions:
// - `/btw` is its own capability so harnesses and agents can opt into the
//   side-question command explicitly instead of depending on a generic bucket.
// - The command executes entirely through the `CommandHost` facilities so
//   every host (server full/dev mode, in-process runtime) gets the same
//   behavior from this one implementation (see knowledge/project/commands.md, EVE-543).
// - The side answer reuses the session's merged context, disables tools, and
//   persists nothing, behaving like Claude Code's ephemeral overlay answer.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::command::{
    CommandArg, CommandDescriptor, CommandExecutionContext, CommandResult, CommandSource,
    ExecuteCommandRequest,
};
use crate::command_host::SessionCompletionRequest;
use crate::error::AgentLoopError;
use crate::message::Message;
use async_trait::async_trait;
use std::collections::HashMap;

pub const BTW_CAPABILITY_ID: &str = "btw";

const BTW_COMMAND_NAME: &str = "btw";
const BTW_SYSTEM_PROMPT: &str = "You are answering an ephemeral side question about the current session. Use the existing conversation as context, answer exactly once, and do not call tools or ask follow-up questions.";

pub struct BtwCapability;

#[async_trait]
impl Capability for BtwCapability {
    fn id(&self) -> &str {
        BTW_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "BTW"
    }

    fn description(&self) -> &str {
        "Ephemeral side-question command for the current session."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "BTW",
            "Ефемерна команда для побічних запитань у поточній сесії.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("message-circle")
    }

    fn category(&self) -> Option<&str> {
        Some("System")
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: BTW_COMMAND_NAME.to_string(),
            description:
                "Ask a side question about the current session without interrupting the main task."
                    .to_string(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "question".to_string(),
                description: "The side question to answer.".to_string(),
                required: true,
                suggestions: vec![],
            }],
        }]
    }

    async fn execute_command(
        &self,
        request: &ExecuteCommandRequest,
        ctx: &CommandExecutionContext,
    ) -> crate::error::Result<CommandResult> {
        if request.name != BTW_COMMAND_NAME {
            return Err(AgentLoopError::config(format!(
                "{} cannot execute /{}",
                self.id(),
                request.name
            )));
        }
        let question = request
            .arguments
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AgentLoopError::config("/btw requires a question"))?;

        let turn = ctx.host.turn_context().await?;

        let mut messages = turn.messages;
        let mut side_question = Message::user(question.to_string());
        side_question.controls = request.controls.clone();
        messages.push(side_question);

        let completion_request = SessionCompletionRequest {
            system_prompts: vec![turn.system_prompt, BTW_SYSTEM_PROMPT.to_string()],
            messages,
            controls: request.controls.clone(),
            metadata: HashMap::from([("command".to_string(), BTW_COMMAND_NAME.to_string())]),
        };

        // Provider/runtime errors are classified and returned as
        // `success: false` (the frontend localizes via `error_code`) instead
        // of bubbling up as hard errors.
        match ctx.host.completion(completion_request).await {
            Ok(completion) => Ok(CommandResult {
                success: true,
                message: completion.text,
                error_code: None,
                error_fields: None,
            }),
            Err(error) => error.into_command_result(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_host::{
        CommandHost, CommandTurnContext, SessionCompletion, SessionCompletionError,
    };
    use crate::session::{Session, SessionStatus};
    use crate::typed_id::{HarnessId, SessionId};
    use crate::user_facing_error::UserFacingErrorContext;
    use std::sync::{Arc, Mutex};

    // Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_btw_capability_registers_command() {
        let cap = BtwCapability;
        let commands = cap.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "btw");
        assert_eq!(commands[0].source, CommandSource::System);
        assert_eq!(commands[0].args.len(), 1);
        assert_eq!(commands[0].args[0].name, "question");
        assert!(commands[0].args[0].required);
    }

    fn test_session(session_id: SessionId) -> Session {
        Session {
            source: Default::default(),
            activity: Default::default(),
            id: session_id,
            workspace_id: crate::WorkspaceId::from_uuid((session_id).uuid()),
            organization_id: crate::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id: HarnessId::new(),
            agent_id: None,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: crate::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: None,
            goal: None,
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: SessionStatus::Started,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
            parent_session_id: None,
            forked_from_session_id: None,
            forked_from_sequence: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    struct StubHost {
        completion_result:
            Mutex<Option<std::result::Result<SessionCompletion, SessionCompletionError>>>,
        seen_request: Mutex<Option<SessionCompletionRequest>>,
    }

    impl StubHost {
        fn returning(
            result: std::result::Result<SessionCompletion, SessionCompletionError>,
        ) -> Arc<Self> {
            Arc::new(Self {
                completion_result: Mutex::new(Some(result)),
                seen_request: Mutex::new(None),
            })
        }
    }

    #[async_trait]
    impl CommandHost for StubHost {
        async fn turn_context(&self) -> crate::error::Result<CommandTurnContext> {
            let session_id = SessionId::new();
            Ok(CommandTurnContext {
                session: test_session(session_id),
                messages: vec![Message::user("earlier message")],
                system_prompt: "merged system prompt".to_string(),
                model: "llmsim-model".to_string(),
                provider_type: "llmsim".to_string(),
                resolved_locale: None,
            })
        }

        async fn completion(
            &self,
            request: SessionCompletionRequest,
        ) -> std::result::Result<SessionCompletion, SessionCompletionError> {
            *self.seen_request.lock().unwrap() = Some(request);
            self.completion_result.lock().unwrap().take().unwrap()
        }
    }

    fn execution_context(host: Arc<StubHost>) -> CommandExecutionContext {
        CommandExecutionContext::new(SessionId::new(), host)
    }

    fn btw_request(arguments: Option<&str>) -> ExecuteCommandRequest {
        ExecuteCommandRequest {
            name: "btw".to_string(),
            arguments: arguments.map(str::to_string),
            controls: None,
        }
    }

    #[tokio::test]
    async fn execute_command_answers_with_session_context() {
        let host = StubHost::returning(Ok(SessionCompletion {
            text: "the side answer".to_string(),
        }));
        let ctx = execution_context(host.clone());

        let result = BtwCapability
            .execute_command(&btw_request(Some("what changed?")), &ctx)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.message, "the side answer");

        let request = host.seen_request.lock().unwrap().take().unwrap();
        // Merged session prompt first, then the btw instruction.
        assert_eq!(request.system_prompts.len(), 2);
        assert_eq!(request.system_prompts[0], "merged system prompt");
        assert!(request.system_prompts[1].contains("ephemeral side question"));
        // History plus the appended side question.
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[1].text(), Some("what changed?"));
        assert_eq!(
            request.metadata.get("command").map(String::as_str),
            Some("btw")
        );
    }

    #[tokio::test]
    async fn execute_command_requires_a_question() {
        let host = StubHost::returning(Ok(SessionCompletion {
            text: "unused".to_string(),
        }));
        let ctx = execution_context(host);

        for arguments in [None, Some(""), Some("   ")] {
            let error = BtwCapability
                .execute_command(&btw_request(arguments), &ctx)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("requires a question"));
        }
    }

    #[tokio::test]
    async fn execute_command_classifies_provider_errors() {
        let host = StubHost::returning(Err(SessionCompletionError::Completion {
            error: "OpenAI API error (429): rate limit exceeded".to_string(),
            context: UserFacingErrorContext::default().with_provider("openai"),
        }));
        let ctx = execution_context(host);

        let result = BtwCapability
            .execute_command(&btw_request(Some("what changed?")), &ctx)
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(result.error_code.as_deref(), Some("provider_rate_limited"));
    }

    #[tokio::test]
    async fn execute_command_bubbles_invalid_completion_requests() {
        let host = StubHost::returning(Err(SessionCompletionError::InvalidRequest(
            AgentLoopError::config("Model not found: model_123"),
        )));
        let ctx = execution_context(host);

        let error = BtwCapability
            .execute_command(&btw_request(Some("what changed?")), &ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Model not found"));
    }

    #[tokio::test]
    async fn execute_command_fails_without_host_facilities() {
        let ctx = CommandExecutionContext::without_host(SessionId::new());

        let error = BtwCapability
            .execute_command(&btw_request(Some("what changed?")), &ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("turn-context"));
    }
}
