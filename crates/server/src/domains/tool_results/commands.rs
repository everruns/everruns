use super::queries as q;
use super::types::{ClientToolResult, SubmitToolResultsResponse};
use crate::domains::common::*;
use crate::storage::models::ClaimWaitingTurnResult;
use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
use everruns_core::message::ContentPart;
use everruns_provider::typed_id::{MessageId, TurnId};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubmitToolResults {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub tool_results: Vec<ClientToolResult>,
}

impl Command for SubmitToolResults {
    type Output = SubmitToolResultsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "submit_tool_results",
            category: "tool_results",
            description: "Submit client-side tool results back to a waiting session.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tool-results",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SubmitToolResultsResponse, CommandError> {
        if self.tool_results.is_empty() {
            return Err(CommandError::bad_request("tool_results must not be empty"));
        }

        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;
        match ctx
            .db
            .claim_waiting_turn(ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
        {
            ClaimWaitingTurnResult::Claimed => {}
            ClaimWaitingTurnResult::Conflict { current_status } => {
                return Err(CommandError::conflict(format!(
                    "Session is not waiting for tool results (current status: {current_status})"
                )));
            }
            ClaimWaitingTurnResult::SessionNotFound => {
                return Err(CommandError::not_found("Session"));
            }
        }

        let turn_id = TurnId::from_uuid(session_id.uuid());
        let event_message_id = MessageId::from_uuid(session_id.uuid());
        let accepted = self.tool_results.len();

        let mut durable_resume_enqueued = false;
        let result: Result<SubmitToolResultsResponse, CommandError> = async {
            for client_result in &self.tool_results {
                let tool_result = if let Some(error) = &client_result.error {
                    ToolCompletedData::failure(
                        client_result.tool_call_id.clone(),
                        String::new(),
                        "error".to_string(),
                        error.clone(),
                        None,
                    )
                } else {
                    let result_content = client_result
                        .result
                        .as_ref()
                        .map(|result| vec![ContentPart::text(result.to_string())])
                        .unwrap_or_default();
                    ToolCompletedData::success(
                        client_result.tool_call_id.clone(),
                        String::new(),
                        result_content,
                        None,
                    )
                };
                q::event_service(ctx)?
                    .emit(EventRequest::new(
                        session_id,
                        EventContext::turn(turn_id, event_message_id),
                        tool_result,
                    ))
                    .await
                    .map_err(classify_anyhow)?;
            }
            q::runner(ctx)?
                .resume_after_tool_results(session_id)
                .await
                .map_err(classify_anyhow)?;
            durable_resume_enqueued = true;
            let completed = ctx
                .db
                .complete_waiting_turn_claim(ctx.org_id(), session_id)
                .await
                .map_err(classify_anyhow)?;
            if !completed {
                return Err(CommandError::internal(anyhow::anyhow!(
                    "waiting-turn claim was lost before activation"
                )));
            }
            Ok(SubmitToolResultsResponse {
                accepted,
                status: "active".to_string(),
            })
        }
        .await;

        if result.is_err()
            && !durable_resume_enqueued
            && let Err(error) = ctx
                .db
                .release_waiting_turn_claim(ctx.org_id(), session_id)
                .await
        {
            tracing::warn!(session_id = %session_id, error = %error, "Failed to release waiting-turn claim");
        }
        result
    }
}

inventory::submit! { CommandDescriptor::of::<SubmitToolResults>() }
