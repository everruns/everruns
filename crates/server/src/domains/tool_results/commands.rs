use super::queries as q;
use super::types::{ClientToolResult, SubmitToolResultsResponse};
use crate::domains::common::*;
use crate::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::models::{ClaimWaitingTurnResult, WaitingTurnResolutionPlan};
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

        let turn_id = TurnId::from_uuid(session_id.uuid());
        let event_message_id = MessageId::from_uuid(session_id.uuid());
        let accepted = self.tool_results.len();
        let events = self
            .tool_results
            .iter()
            .map(|client_result| {
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
                EventRequest::new(
                    session_id,
                    EventContext::turn(turn_id, event_message_id),
                    tool_result,
                )
            })
            .collect();
        let plan = WaitingTurnResolutionPlan {
            kind: "tool_results".to_string(),
            events,
            session_values: Vec::new(),
            response: serde_json::json!({ "accepted": accepted }),
        };
        let claim = match ctx
            .db
            .claim_waiting_turn(ctx.org_id(), session_id, plan)
            .await
            .map_err(classify_anyhow)?
        {
            ClaimWaitingTurnResult::Claimed(claim) => claim,
            ClaimWaitingTurnResult::Conflict { current_status } => {
                return Err(CommandError::conflict(format!(
                    "Session is not waiting for tool results (current status: {current_status})"
                )));
            }
            ClaimWaitingTurnResult::SessionNotFound => {
                return Err(CommandError::not_found("Session"));
            }
        };
        let accepted = claim.plan.response["accepted"]
            .as_u64()
            .unwrap_or(accepted as u64) as usize;
        execute_waiting_turn_resolution(
            &ctx.db,
            q::event_service(ctx)?,
            &q::runner(ctx)?,
            ctx.org_id(),
            session_id,
            &claim,
        )
        .await
        .map_err(classify_anyhow)?;
        Ok(SubmitToolResultsResponse {
            accepted,
            status: "active".to_string(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<SubmitToolResults>() }
