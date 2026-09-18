//! Budgets, rate limits, payments, and session authorization.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_check_budgets_for_session(
        &self,
        request: Request<CheckBudgetsForSessionRequest>,
    ) -> Result<Response<CheckBudgetsForSessionResponse>, Status> {
        let req = request.into_inner();

        let all_budgets = self
            .budget_service
            .list_budgets_for_session_hierarchy(
                req.org_id,
                &req.session_id,
                req.agent_id.as_deref(),
            )
            .await;

        if all_budgets.is_empty() {
            return Ok(Response::new(CheckBudgetsForSessionResponse {
                status: "no_budgets".into(),
                budgets: vec![],
                hint: Some(
                    "No budgets are configured for this session. You can proceed without budget constraints.".into(),
                ),
            }));
        }

        // Also run the action check to determine overall status
        let check_result = self
            .budget_service
            .check_budgets_for_session(req.org_id, &req.session_id, req.agent_id.as_deref())
            .await;

        let summaries: Vec<BudgetSummaryProto> = all_budgets
            .iter()
            .map(|b| {
                let pct = if b.limit > 0.0 {
                    (b.balance / b.limit * 100.0).clamp(0.0, 100.0)
                } else {
                    100.0
                };
                BudgetSummaryProto {
                    currency: b.currency.clone(),
                    limit: b.limit,
                    balance: b.balance,
                    soft_limit: b.soft_limit,
                    percent_remaining: (pct * 10.0).round() / 10.0,
                    status: b.status.clone(),
                }
            })
            .collect();

        let overall_status = match check_result.action.as_str() {
            "stop" => "exhausted",
            "pause" => "paused",
            "warn" => "warning",
            _ => "active",
        };

        let hint = if overall_status == "active" {
            let min_pct = summaries
                .iter()
                .map(|s| s.percent_remaining)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(100.0);
            if min_pct < 50.0 {
                Some(format!(
                    "{min_pct}% of budget remaining. Consider prioritizing task completion."
                ))
            } else {
                Some(format!("{min_pct}% of budget remaining."))
            }
        } else {
            check_result.message.clone()
        };

        Ok(Response::new(CheckBudgetsForSessionResponse {
            status: overall_status.into(),
            budgets: summaries,
            hint,
        }))
    }

    pub(crate) async fn handle_check_outbound_tool_rate_limit(
        &self,
        request: Request<CheckOutboundToolRateLimitRequest>,
    ) -> Result<Response<CheckOutboundToolRateLimitResponse>, Status> {
        let req = request.into_inner();
        let allowed = match &self.org_rate_limiter {
            Some(limiter) => limiter.check_outbound_tool_call(&req.org_key).await.is_ok(),
            None => {
                tracing::error!(
                    org_key = %req.org_key,
                    "gRPC outbound tool rate limiter is not configured; denying tool call"
                );
                false
            }
        };

        Ok(Response::new(CheckOutboundToolRateLimitResponse {
            allowed,
        }))
    }

    pub(crate) async fn handle_execute_machine_payment(
        &self,
        request: Request<ExecuteMachinePaymentRequest>,
    ) -> Result<Response<ExecuteMachinePaymentResponse>, Status> {
        use everruns_core::payment::{MachinePaymentRequest, PaymentMethod, PaymentRail};
        use everruns_core::tool_execution::PaymentAuthority;
        use everruns_provider::typed_id::{AgentId, SessionId};

        let req = request.into_inner();
        let session_id = SessionId::parse(&req.session_id)
            .or_else(|_| uuid::Uuid::parse_str(&req.session_id).map(SessionId::from_uuid))
            .map_err(|error| Status::invalid_argument(format!("Invalid session_id: {error}")))?;
        let agent_id = req
            .agent_id
            .as_deref()
            .map(|value| {
                AgentId::parse(value)
                    .or_else(|_| uuid::Uuid::parse_str(value).map(AgentId::from_uuid))
            })
            .transpose()
            .map_err(|error| Status::invalid_argument(format!("Invalid agent_id: {error}")))?;
        let method: PaymentMethod = req.method.parse().map_err(|error| {
            Status::invalid_argument(format!("Invalid payment method: {error}"))
        })?;
        let rail_preference = req
            .rail_preference
            .iter()
            .map(|rail| rail.parse::<PaymentRail>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Status::invalid_argument(format!("Invalid payment rail: {error}")))?;
        let body = req
            .body
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json);
        let metadata = req
            .metadata
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or_else(|| serde_json::json!({}));

        let authority = crate::domains::payments::ServerPaymentAuthority::new(
            self.db.clone(),
            self.encryption.clone(),
            req.org_id,
            agent_id,
        );
        let response = authority
            .execute_machine_payment(
                session_id,
                MachinePaymentRequest {
                    capability: req.capability,
                    operation: req.operation,
                    method,
                    url: req.url,
                    body,
                    max_amount_usd: req.max_amount_usd,
                    rail_preference,
                    metadata,
                },
            )
            .await
            .map_err(payment_error_to_status)?;

        Ok(Response::new(ExecuteMachinePaymentResponse {
            attempt_id: response.attempt_id.map(|id| id.to_string()),
            amount_usd: response.amount_usd,
            rail: response.rail.map(|rail| rail.to_string()),
            response: Some(everruns_internal_protocol::json_to_proto_value(
                &response.response,
            )),
            receipt: Some(everruns_internal_protocol::json_to_proto_value(
                &response.receipt,
            )),
        }))
    }

    pub(crate) async fn handle_authorize_session_creation(
        &self,
        request: Request<AuthorizeSessionCreationRequest>,
    ) -> Result<Response<AuthorizeSessionCreationResponse>, Status> {
        // THREAT[TM-AUTHZ-014]: Resolve the current session owner server-side;
        // the worker cannot supply a user identity for this decision.
        let req = request.into_inner();
        let session_id = everruns_provider::typed_id::SessionId::parse(&req.session_id)
            .or_else(|_| {
                uuid::Uuid::parse_str(&req.session_id)
                    .map(everruns_provider::typed_id::SessionId::from_uuid)
            })
            .map_err(|error| Status::invalid_argument(format!("Invalid session_id: {error}")))?;
        let session = self
            .db
            .get_session(req.org_id, session_id)
            .await
            .map_err(|error| internal_status("Failed to load session", error))?
            .ok_or_else(|| Status::not_found("Session not found"))?;
        let user_id = session.resolved_owner_user_id.ok_or_else(|| {
            Status::permission_denied(
                "Detached session creation requires a user-owned session with a resolved owner",
            )
        })?;
        let caller = crate::auth::caller_resolution::caller_for_user(&self.db, req.org_id, user_id)
            .await
            .map_err(|error| {
                // THREAT[TM-API-005]: the resolver failure is a server-side diagnostic;
                // the worker only needs to know the decision.
                tracing::error!(%error, "Failed to resolve session owner");
                Status::permission_denied("Failed to resolve session owner")
            })?;
        crate::domains::sessions::SESSION_MANAGE
            .evaluate_with(self.permission_resolver.as_ref(), &caller)
            .map_err(|error| Status::permission_denied(error.message))?;
        let root_session_id = session.root_session_id.unwrap_or(session.id);
        Ok(Response::new(AuthorizeSessionCreationResponse {
            budget_root_session_id: root_session_id.to_string(),
        }))
    }
}
