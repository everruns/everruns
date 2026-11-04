use super::queries as q;
use super::types::{ClientToolResult, SubmitToolResultsResponse};
use crate::domains::common::*;
use crate::domains::tool_results::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::ClaimWaitingTurnResult;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct SubmitToolResults {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Results for the pending client-side tool calls, one entry per `tool_call_id`.
    pub tool_results: Vec<ClientToolResult>,
}

#[command(
    name = "submit_tool_results",
    category = "tool_results",
    description = "Submit client-side tool results back to a waiting session.",
    method = "POST",
    cli = CliRoute::new(&["sessions", "tool-results"], "submit").with_args(&[CliArg::new("session_id").at(1)]).with_examples(&[CliExample::new("Return the result of a client-side tool call to a waiting session", "everruns sessions tool-results submit session_01h9 --tool-results '[{\"tool_call_id\":\"toolu_01\",\"result\":{\"url\":\"https://example.com/orders/42\"}}]' --reason 'Deliver the tool output'")]),
    path = "/v1/sessions/{session_id}/tool-results",
    policy = crate::domains::sessions::SESSION_MANAGE,
    positional = "session_id",
)]
impl Command for SubmitToolResults {
    type Output = SubmitToolResultsResponse;

    async fn execute(self, ctx: &Ctx) -> Result<SubmitToolResultsResponse, CommandError> {
        if self.tool_results.is_empty() {
            return Err(CommandError::bad_request("tool_results must not be empty"));
        }

        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        // Recorded under the waiting turn, so the worker's fold settles them.
        let accepted = self.tool_results.len();
        let results = self
            .tool_results
            .iter()
            .map(ClientToolResult::completed)
            .collect();
        let claim = match ctx
            .db
            .claim_tool_result_command(ctx.org_id(), session_id, results)
            .await
            .map_err(|error| {
                if let Some(error) = error.downcast_ref::<everruns_contracts::AgentLoopError>() {
                    CommandError::bad_request(error.to_string())
                } else {
                    CommandError::from(error)
                }
            })? {
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
        .await?;
        Ok(SubmitToolResultsResponse {
            accepted,
            status: if claim
                .plan
                .response
                .get("resume")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
            {
                "waiting_for_tool_results"
            } else {
                "active"
            }
            .to_string(),
        })
    }
}
