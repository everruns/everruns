// Credit shortfall: what the engine does when the provider refuses a request
// because the account cannot pay for it (knowledge/execution/credit-shortfall.md).
//
// Decisions:
// - Shrink first. A refusal that names how many output tokens the balance
//   still covers (OpenRouter: "can only afford N") is retried once with max
//   output tokens lowered to that, minus a margin, when N is at least
//   `SHRINK_MIN_TOKENS`. Below that an answer would be cut short, so the turn
//   waits instead. The retry is safe: a 402 arrives before any output.
// - Wait without a worker. The turn fails as before, and a one-shot session
//   schedule (the durable timer the rate-limit retry and usage-limit
//   continuation already use) fires a check every `PROBE_INTERVAL_SECS`. The
//   check is any reason before the model answers the check: it reads the provider's
//   balance (`ChatDriver::credit_status`, no tokens spent) before any model
//   call. Still out: it schedules the next check and ends quietly, without
//   calling the model. Back: it cancels any other queued check and the turn
//   continues the work. A provider with no balance API is checked by the
//   request itself, on the same timer and in the same window.
// - The deadline travels in the check's own prompt ("... Waiting until
//   <RFC 3339>."), so every check of one wait shares it without any state
//   outside the schedule rows: a one-shot schedule keeps no end time.
// - Durable execution: nothing lives in memory. A check fires once (the
//   scheduler claims a one-shot row under a lease). A check replayed after a
//   worker restart finds the next check already queued and does not add a
//   second, and a resume cancels queued checks, so a wait never resumes twice.
//   A resumed request the provider still refuses goes back to waiting until
//   the deadline, which bounds the whole wait.
// - Stand down: when the next check would land past the deadline, the turn
//   fails with the ordinary `provider_quota_exhausted` error and no new wait
//   starts from it.

use chrono::{DateTime, Duration, SecondsFormat, SubsecRound, Utc};

use crate::engine::error::{AgentLoopError, LlmErrorKind};
use crate::engine::event_emitter::EventEmitter;
use crate::engine::events::{
    CreditShortfallAction, EventContext, EventRequest, TurnCreditShortfallData,
};
use crate::engine::message::{RuntimeMessage, RuntimeMessageRole};
use crate::engine::session_services::SessionScheduleStore;
use crate::engine::typed_id::{MessageId, SessionId, TurnId};
use crate::engine::{UserFacingError, UserFacingErrorContext, user_facing_error_codes};
use everruns_contracts::message_origin::MessageOriginKind;
use everruns_contracts::provider_limit::CreditStatus;

/// Smallest balance, in output tokens, worth a smaller retry.
pub const SHRINK_MIN_TOKENS: u64 = 4096;

/// Tokens kept under what the provider said the balance covers, since input
/// tokens and price rounding are charged from the same balance.
const SHRINK_MARGIN_TOKENS: u64 = 512;

/// Seconds between two checks for credits.
pub const PROBE_INTERVAL_SECS: i64 = 60;

/// How long a turn waits for credits before it stands down.
pub const WAIT_WINDOW_SECS: i64 = 3_600;

/// Prompt of a credit check. Also the marker that tells a check's schedule
/// and turn apart from others; the deadline follows it.
pub const CREDIT_WAIT_PROMPT: &str =
    "Continue the interrupted work (automatic check: the AI provider account ran out of credits).";

const UNTIL: &str = " Waiting until ";

/// The refusal a smaller retry answers: (requested, affordable, retry max).
pub(crate) fn shrink_target(error: &AgentLoopError) -> Option<(u64, u64, u32)> {
    let classified = error.user_facing_error(UserFacingErrorContext::default());
    let (requested, affordable) = affordability(&classified)?;
    if affordable < SHRINK_MIN_TOKENS || affordable >= requested {
        return None;
    }
    let max = u32::try_from(affordable.saturating_sub(SHRINK_MARGIN_TOKENS)).ok()?;
    Some((requested, affordable, max))
}

fn affordability(error: &UserFacingError) -> Option<(u64, u64)> {
    if error.code != user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED {
        return None;
    }
    let field = |key: &str| error.fields.get(key).and_then(serde_json::Value::as_u64);
    Some((field("requested_max_tokens")?, field("affordable_tokens")?))
}

/// The prompt of a check in a wait that ends at `deadline`.
pub fn check_prompt(deadline: DateTime<Utc>) -> String {
    format!(
        "{CREDIT_WAIT_PROMPT}{UNTIL}{}.",
        deadline.to_rfc3339_opts(SecondsFormat::Secs, true)
    )
}

fn parse_deadline(text: &str) -> Option<DateTime<Utc>> {
    let rest = text.strip_prefix(CREDIT_WAIT_PROMPT)?.strip_prefix(UNTIL)?;
    DateTime::parse_from_rfc3339(rest.strip_suffix('.')?)
        .ok()
        .map(|deadline| deadline.with_timezone(&Utc))
}

/// A turn started by a credit check, with its wait's deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CreditCheck {
    pub deadline: DateTime<Utc>,
}

/// The credit check this turn runs, when its input is one and the model has
/// not answered it yet. Keyed on the transcript, not the iteration: a check
/// recovered after a worker restart can come back as a later iteration, and
/// must read the balance again rather than call the model.
pub(crate) fn credit_check(
    messages: &[RuntimeMessage],
    input_message_id: MessageId,
) -> Option<CreditCheck> {
    let position = messages
        .iter()
        .position(|message| message.id == input_message_id)?;
    let input = &messages[position];
    if messages[position + 1..]
        .iter()
        .any(|message| message.role == RuntimeMessageRole::Agent)
    {
        return None;
    }
    if input.origin()?.kind != MessageOriginKind::Schedule {
        return None;
    }
    parse_deadline(input.text()?.trim()).map(|deadline| CreditCheck { deadline })
}

/// What scheduling the next check came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// A check is scheduled at `next`.
    Scheduled {
        deadline: DateTime<Utc>,
        next: DateTime<Utc>,
    },
    /// A check is already queued (a replayed check, or failures racing).
    Queued { deadline: DateTime<Utc> },
    /// The next check would land past the deadline.
    Expired { deadline: DateTime<Utc> },
    /// No schedule could be made; the turn fails as it always did.
    Unavailable,
}

/// Schedule the next check of the wait `check` belongs to, or start a wait.
pub(crate) async fn schedule_check(
    store: &dyn SessionScheduleStore,
    session_id: SessionId,
    check: Option<CreditCheck>,
    now: DateTime<Utc>,
) -> Wait {
    // Whole seconds: the deadline is read back from the check's prompt.
    let deadline = check.map_or(
        (now + Duration::seconds(WAIT_WINDOW_SECS)).trunc_subsecs(0),
        |check| check.deadline,
    );
    let schedules = match store.list_schedules(session_id).await {
        Ok(schedules) => schedules,
        Err(error) => {
            tracing::warn!(%session_id, %error, "credit wait: could not list schedules");
            return Wait::Unavailable;
        }
    };
    if let Some(queued) = schedules
        .iter()
        .filter(|s| s.enabled && s.trigger_count == 0 && s.cron_expression.is_none())
        .find_map(|s| parse_deadline(&s.description))
    {
        return Wait::Queued { deadline: queued };
    }
    let next = now + Duration::seconds(PROBE_INTERVAL_SECS);
    if next > deadline {
        return Wait::Expired { deadline };
    }
    match store
        .create_schedule_enforcing_limits(
            session_id,
            check_prompt(deadline),
            None,
            Some(next),
            "UTC".to_string(),
            Default::default(),
        )
        .await
    {
        Ok(_) => Wait::Scheduled { deadline, next },
        Err(_) => {
            tracing::warn!(%session_id, "credit wait: schedule limits refused the check");
            Wait::Unavailable
        }
    }
}

/// Cancel checks still queued, once a check resumed the turn.
pub(crate) async fn cancel_queued_checks(store: &dyn SessionScheduleStore, session_id: SessionId) {
    let Ok(schedules) = store.list_schedules(session_id).await else {
        return;
    };
    for schedule in schedules
        .iter()
        .filter(|s| s.enabled && s.trigger_count == 0 && parse_deadline(&s.description).is_some())
    {
        if let Err(error) = store.cancel_schedule(session_id, schedule.id).await {
            tracing::warn!(%session_id, %error, "credit wait: could not cancel a queued check");
        }
    }
}

/// The provider's balance, read without spending tokens. A failed read counts
/// as still out: the wait goes on, bounded by its deadline.
pub(crate) async fn read_balance(driver: &dyn crate::engine::ChatDriver) -> Option<CreditStatus> {
    match driver
        .credit_status(&crate::engine::ProviderEndpoint::default())
        .await
    {
        Ok(status) => status,
        Err(error) => {
            tracing::warn!(%error, "credit wait: could not read the provider balance");
            Some(CreditStatus::Exhausted)
        }
    }
}

/// The error a wait that ran out fails its turn with: the ordinary quota error.
pub(crate) fn gave_up_error() -> AgentLoopError {
    AgentLoopError::llm_kind(
        LlmErrorKind::QuotaExhausted,
        "AI provider credits did not come back before the wait ended",
    )
}

/// Record one credit decision on the turn.
pub(crate) struct Record<'a> {
    pub emitter: &'a dyn EventEmitter,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub input_message_id: MessageId,
}

impl Record<'_> {
    pub(crate) fn data(&self, action: CreditShortfallAction) -> TurnCreditShortfallData {
        TurnCreditShortfallData {
            turn_id: self.turn_id,
            action,
            requested_max_tokens: None,
            affordable_tokens: None,
            max_tokens: None,
            wait_until: None,
            next_check_at: None,
            balance_checked: None,
        }
    }

    pub(crate) async fn emit(&self, data: TurnCreditShortfallData) {
        let request = EventRequest::new(
            self.session_id,
            EventContext::turn(self.turn_id, self.input_message_id),
            data,
        );
        if let Err(error) = self.emitter.emit(request).await {
            tracing::warn!(session_id = %self.session_id, %error, "credit shortfall: event not recorded");
        }
    }
}

/// Fill a record from the refused request's error fields.
pub(crate) fn with_refusal(
    mut data: TurnCreditShortfallData,
    error: &UserFacingError,
) -> TurnCreditShortfallData {
    let field = |key: &str| error.fields.get(key).and_then(serde_json::Value::as_u64);
    data.requested_max_tokens = field("requested_max_tokens");
    data.affordable_tokens = field("affordable_tokens");
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFFORD: &str = "OpenRouter API error (402): This request requires more credits, or fewer max_tokens. You requested up to 65536 tokens, but can only afford";

    fn refusal(affordable: u64) -> AgentLoopError {
        AgentLoopError::llm_kind(
            LlmErrorKind::QuotaExhausted,
            format!("{AFFORD} {affordable}."),
        )
    }

    #[test]
    fn a_balance_that_covers_a_useful_answer_shrinks_below_it() {
        assert_eq!(
            shrink_target(&refusal(20_000)),
            Some((65_536, 20_000, 20_000 - 512))
        );
        assert_eq!(shrink_target(&refusal(SHRINK_MIN_TOKENS - 1)), None);
        assert_eq!(shrink_target(&refusal(283)), None);
        // No numbers: nothing to shrink to.
        assert_eq!(
            shrink_target(&AgentLoopError::llm_kind(
                LlmErrorKind::QuotaExhausted,
                "insufficient_quota"
            )),
            None
        );
    }

    #[test]
    fn the_check_prompt_carries_its_deadline() {
        let deadline = DateTime::parse_from_rfc3339("2026-11-03T15:04:05Z")
            .unwrap()
            .with_timezone(&Utc);
        let prompt = check_prompt(deadline);
        assert!(
            prompt.ends_with("Waiting until 2026-11-03T15:04:05Z."),
            "{prompt}"
        );
        assert_eq!(parse_deadline(&prompt), Some(deadline));
        assert_eq!(parse_deadline(CREDIT_WAIT_PROMPT), None);
        assert_eq!(parse_deadline("Daily report"), None);
    }
}
