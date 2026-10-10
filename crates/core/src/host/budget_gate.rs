//! Budget gate applied before every provider call.
//!
//! Design decisions:
//! - Budgets are debited post-hoc, after each `llm.generation` (the server's
//!   `BudgetService`). Nothing in the debit path can stop a turn, so the loop
//!   reads the budget status before each reason atom instead. One checker call
//!   per reason, which bounds overspend to the provider call already in flight.
//! - One gate for every path that runs a reason: the in-process loop, the
//!   durable workflow, and worker activities all go through
//!   `execute_reason_activity_with_prompt_messages`; the OpenAI Agents API
//!   backend reuses the same evaluation before each tool batch it runs.
//! - A checker error fails open (warn and continue), as everywhere budgets are
//!   read: an unreachable budget store must not take every turn down.
//! - A stop ends the turn through the ordinary failed-reason path, so the
//!   engine emits exactly one `turn.failed` with the `budget_exhausted` /
//!   `budget_paused` code. The gate itself emits `budget.exhausted` /
//!   `budget.paused`, and `budget.warning` at most once per turn (on its first
//!   iteration): the debit path has no event emitter to raise it there.

use crate::budget::{BudgetSummary, BudgetToolResponse};
use crate::engine::{ReasonInput, ReasonResult};
use crate::events::{BUDGET_EXHAUSTED, BUDGET_PAUSED, BUDGET_WARNING, BudgetEventData, EventData};
use crate::events::{EventContext, EventRequest, OutputMessageCompletedData};
use crate::message::RuntimeMessage;
use everruns_contracts::typed_id::{AgentId, SessionId};
use everruns_contracts::user_facing_error::{UserFacingError, codes};

/// What the gate decided for one budget reading.
#[derive(Debug, Clone)]
pub(crate) enum BudgetGate {
    Continue,
    /// Running low: keep going, but say so.
    Warn(BudgetEventData),
    /// Exhausted or paused: the turn ends before the next provider call.
    Stop(BudgetStop),
}

#[derive(Debug, Clone)]
pub(crate) struct BudgetStop {
    /// `budget.exhausted` or `budget.paused`.
    pub(crate) event_type: &'static str,
    pub(crate) event: BudgetEventData,
    /// Carries the stable `budget_exhausted` / `budget_paused` code.
    pub(crate) user_error: UserFacingError,
}

impl BudgetStop {
    pub(crate) fn code(&self) -> &str {
        &self.user_error.code
    }

    /// The canonical user-facing copy for this stop.
    pub(crate) fn message(&self) -> String {
        self.user_error.fallback_message()
    }

    pub(crate) fn event_data(&self) -> EventData {
        EventData::budget_event(self.event.clone(), self.event_type)
    }
}

/// Read the session's budgets through the host's checker. `None` when the
/// host has no checker, or the checker failed (fail open).
pub(crate) async fn read_budgets<A: crate::host::RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    agent_id: Option<AgentId>,
    session_id: SessionId,
) -> Option<BudgetToolResponse> {
    let checker = adapter.budget_checker(org_id, agent_id)?;
    let session_id = session_id.to_string();
    match checker.check_budgets(&session_id).await {
        Ok(response) => Some(response),
        Err(error) => {
            tracing::warn!(%error, %session_id, "budget check failed; continuing");
            None
        }
    }
}

/// Decide what a budget reading means for the next provider call.
pub(crate) fn evaluate(response: &BudgetToolResponse) -> BudgetGate {
    match response.status.as_str() {
        "exhausted" => {
            let summary = pick(response, |b| b.status == "exhausted" || b.balance <= 0.0);
            let mut user_error = UserFacingError::new(codes::BUDGET_EXHAUSTED);
            if let Some(b) = summary {
                user_error = with_budget_id(user_error, b)
                    .with_field("spent", spent(b))
                    .with_field("limit", b.limit)
                    .with_field("currency", b.currency.clone());
            }
            BudgetGate::Stop(BudgetStop {
                event_type: BUDGET_EXHAUSTED,
                event: event_data(summary, user_error.fallback_message()),
                user_error,
            })
        }
        "paused" => {
            let summary = pick(response, |b| {
                b.status == "paused" || b.soft_limit.is_some_and(|soft| spent(b) >= soft)
            });
            let mut user_error = UserFacingError::new(codes::BUDGET_PAUSED);
            if let Some(b) = summary {
                user_error = with_budget_id(user_error, b)
                    .with_field("spent", spent(b))
                    .with_optional_field("soft_limit", b.soft_limit)
                    .with_field("currency", b.currency.clone());
            }
            BudgetGate::Stop(BudgetStop {
                event_type: BUDGET_PAUSED,
                event: event_data(summary, user_error.fallback_message()),
                user_error,
            })
        }
        "warning" => {
            let summary = pick(response, |_| false);
            let message = summary.map_or_else(
                || "Budget running low.".to_string(),
                |b| {
                    format!(
                        "Budget running low. {:.2} {} remaining of {:.2} {} limit.",
                        b.balance, b.currency, b.limit, b.currency
                    )
                },
            );
            BudgetGate::Warn(event_data(summary, message))
        }
        _ => BudgetGate::Continue,
    }
}

/// The budget behind the overall status: the first that matches, else the
/// one with the least headroom.
fn pick(
    response: &BudgetToolResponse,
    matches: impl Fn(&BudgetSummary) -> bool,
) -> Option<&BudgetSummary> {
    response.budgets.iter().find(|b| matches(b)).or_else(|| {
        response
            .budgets
            .iter()
            .min_by(|a, b| a.percent_remaining.total_cmp(&b.percent_remaining))
    })
}

fn spent(budget: &BudgetSummary) -> f64 {
    (budget.limit - budget.balance).max(0.0)
}

fn with_budget_id(error: UserFacingError, budget: &BudgetSummary) -> UserFacingError {
    match &budget.budget_id {
        Some(id) => error.with_field("budget_id", id.clone()),
        None => error,
    }
}

fn event_data(summary: Option<&BudgetSummary>, message: String) -> BudgetEventData {
    BudgetEventData {
        budget_id: summary
            .and_then(|b| b.budget_id.clone())
            .unwrap_or_default(),
        balance: summary.map_or(0.0, |b| b.balance),
        limit: summary.map_or(0.0, |b| b.limit),
        currency: summary.map(|b| b.currency.clone()).unwrap_or_default(),
        message: Some(message),
        soft_limit: summary.and_then(|b| b.soft_limit),
    }
}

/// The gate the native loop applies before a reason atom. `Some` ends the
/// turn: the returned failed result carries the budget error, after the
/// budget event and the user-visible message have been emitted.
pub(crate) async fn before_reason<A: crate::host::RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: &ReasonInput,
) -> everruns_contracts::error::Result<Option<ReasonResult>> {
    let Some(response) =
        read_budgets(adapter, org_id, input.agent_id, input.context.session_id).await
    else {
        return Ok(None);
    };
    let context = EventContext::turn(input.context.turn_id, input.context.input_message_id);
    let emitter = adapter.event_emitter();
    match evaluate(&response) {
        BudgetGate::Continue => Ok(None),
        BudgetGate::Warn(data) => {
            if input.iteration <= 1 {
                emitter
                    .emit(EventRequest::new(
                        input.context.session_id,
                        context,
                        EventData::budget_event(data, BUDGET_WARNING),
                    ))
                    .await?;
            }
            Ok(None)
        }
        BudgetGate::Stop(stop) => {
            let message = stop.message();
            emitter
                .emit(EventRequest::new(
                    input.context.session_id,
                    context.clone(),
                    stop.event_data(),
                ))
                .await?;
            // Shown in the conversation like any other run-blocking error;
            // the error-code metadata keeps it out of later model context.
            let mut error_message = RuntimeMessage::assistant(message.clone());
            let mut metadata = std::collections::HashMap::new();
            stop.user_error.apply_to_message_metadata(&mut metadata);
            error_message.metadata = Some(metadata);
            emitter
                .emit(EventRequest::new(
                    input.context.session_id,
                    context,
                    OutputMessageCompletedData::new(error_message)
                        .with_user_facing_error(&stop.user_error),
                ))
                .await?;
            Ok(Some(ReasonResult {
                text: message.clone(),
                max_iterations: crate::runtime_agent::default_max_iterations(),
                error: Some(message),
                user_facing_error: Some(stop.user_error),
                ..Default::default()
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(status: &str, balance: f64, limit: f64) -> BudgetSummary {
        BudgetSummary {
            budget_id: Some("bdgt_1".into()),
            currency: "tokens".into(),
            limit,
            balance,
            soft_limit: None,
            percent_remaining: balance / limit * 100.0,
            status: status.into(),
        }
    }

    fn response(status: &str, budgets: Vec<BudgetSummary>) -> BudgetToolResponse {
        BudgetToolResponse {
            status: status.into(),
            budgets,
            hint: None,
        }
    }

    #[test]
    fn exhausted_budget_stops_with_canonical_copy_and_fields() {
        let BudgetGate::Stop(stop) = evaluate(&response(
            "exhausted",
            vec![
                summary("active", 90.0, 100.0),
                summary("exhausted", 0.0, 100.0),
            ],
        )) else {
            panic!("exhausted must stop");
        };
        assert_eq!(stop.code(), codes::BUDGET_EXHAUSTED);
        assert_eq!(stop.event_type, BUDGET_EXHAUSTED);
        assert_eq!(
            stop.message(),
            "Budget exhausted. 100.00 tokens spent reached the 100.00 tokens limit. Increase the budget to continue."
        );
        assert_eq!(stop.event.budget_id, "bdgt_1");
        assert_eq!(stop.event.balance, 0.0);
        assert_eq!(stop.event.limit, 100.0);
    }

    #[test]
    fn paused_budget_stops_with_paused_code() {
        let mut paused = summary("paused", 40.0, 100.0);
        paused.soft_limit = Some(50.0);
        let BudgetGate::Stop(stop) = evaluate(&response("paused", vec![paused])) else {
            panic!("paused must stop");
        };
        assert_eq!(stop.code(), codes::BUDGET_PAUSED);
        assert_eq!(stop.event_type, BUDGET_PAUSED);
        assert_eq!(
            stop.message(),
            "Budget paused. 60.00 tokens spent exceeded the 50.00 tokens soft limit. Increase or resume the budget to continue."
        );
        assert_eq!(stop.event.soft_limit, Some(50.0));
    }

    #[test]
    fn stop_without_budget_details_uses_bare_copy() {
        let BudgetGate::Stop(stop) = evaluate(&response("exhausted", vec![])) else {
            panic!("exhausted must stop");
        };
        assert_eq!(
            stop.message(),
            "Budget exhausted. Increase the budget to continue."
        );
        let BudgetGate::Stop(stop) = evaluate(&response("paused", vec![])) else {
            panic!("paused must stop");
        };
        assert_eq!(
            stop.message(),
            "Budget paused. Increase or resume the budget to continue."
        );
    }

    /// The copy round-trips through the runtime classifier, so persisted
    /// errors (durable task errors, the Agents API tool result) keep the code.
    #[test]
    fn stop_copy_classifies_back_to_its_code() {
        let mut paused = summary("paused", 40.0, 100.0);
        paused.soft_limit = Some(50.0);
        for response in [
            response("exhausted", vec![summary("exhausted", 0.0, 100.0)]),
            response("exhausted", vec![]),
            response("paused", vec![paused]),
            response("paused", vec![]),
        ] {
            let BudgetGate::Stop(stop) = evaluate(&response) else {
                panic!("must stop");
            };
            let classified = everruns_contracts::classify_runtime_error_message(
                &stop.message(),
                &everruns_contracts::UserFacingErrorContext::default(),
            );
            assert_eq!(classified.code, stop.code());
        }
    }

    #[test]
    fn warning_picks_the_budget_with_least_headroom() {
        let BudgetGate::Warn(data) = evaluate(&response(
            "warning",
            vec![
                summary("active", 50.0, 100.0),
                summary("active", 10.0, 100.0),
            ],
        )) else {
            panic!("warning must warn");
        };
        assert_eq!(data.balance, 10.0);
        assert!(data.message.unwrap().starts_with("Budget running low."));
    }

    #[test]
    fn other_statuses_continue() {
        for status in ["active", "no_budgets", "disabled", "unknown"] {
            assert!(matches!(
                evaluate(&response(status, vec![])),
                BudgetGate::Continue
            ));
        }
    }
}

#[cfg(test)]
#[path = "budget_gate_tests.rs"]
mod runtime_tests;
