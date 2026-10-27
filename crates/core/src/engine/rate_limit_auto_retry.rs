// Rate-limit auto-retry: always-on platform error hook
//
// A provider can refuse a turn with a short, known wait: OpenRouter's 402
// "in-flight budget exhausted" asks for `Retry-After: 120`, and a 429 can
// outlast the in-call retry budget. Neither is retried by the durable engine,
// so without this hook the session idles until a person sends another message.
//
// On a terminal `provider_rate_limited` error whose `retry_after` is at most
// `MAX_RETRY_AFTER_SECS`, the hook schedules one continuation at
// `now + retry_after + jitter` through the same schedule store (and limits) as
// `usage_limit_auto_continue`, and sets `auto_retry` so the copy says so.
//
// Unlike that capability, this hook needs no configuration: the reason atom
// appends it to every turn's error hooks (see `collect_llm_error_hooks`).
// Loop guard: at most `MAX_RETRIES_PER_WINDOW` automatic retries per session
// inside `RETRY_WINDOW_SECS`, counted from the schedules it created (they are
// marked by `RATE_LIMIT_RETRY_PROMPT`). A run that keeps hitting the limit
// therefore stops after three tries and waits for a person, as before.

use crate::engine::llm_error_hook::{LlmErrorContext, LlmErrorHook, LlmErrorHookOutcome};
use crate::engine::user_facing_error_codes;
use crate::session_schedule::SessionSchedule;
use async_trait::async_trait;
use rand::RngExt;

/// Message injected by the retry continuation. Also the marker that tells the
/// hook's own schedules apart from ones the agent or a user created.
pub const RATE_LIMIT_RETRY_PROMPT: &str =
    "Continue the interrupted work (automatic retry after an AI provider rate limit).";

/// Longest provider-requested wait the hook schedules a retry for. Anything
/// longer is a quota or plan limit in disguise, not a short throttle.
pub const MAX_RETRY_AFTER_SECS: u64 = 600;

/// Automatic retries allowed per session inside [`RETRY_WINDOW_SECS`].
pub const MAX_RETRIES_PER_WINDOW: usize = 3;

/// Window over which [`MAX_RETRIES_PER_WINDOW`] is counted.
pub const RETRY_WINDOW_SECS: i64 = 3_600;

/// Upper bound of the random delay added to `retry_after`, so sessions refused
/// together do not retry in lockstep.
const MAX_JITTER_SECS: u64 = 15;

/// The always-on hook. Stateless; state lives in the session's schedules.
pub struct RateLimitAutoRetryHook;

#[async_trait]
impl LlmErrorHook for RateLimitAutoRetryHook {
    async fn on_llm_error(&self, ctx: &LlmErrorContext<'_>) -> LlmErrorHookOutcome {
        if ctx.error_code != user_facing_error_codes::PROVIDER_RATE_LIMITED {
            return LlmErrorHookOutcome::noop();
        }
        let Some(retry_after) = ctx.error_fields.get("retry_after").and_then(|v| v.as_u64()) else {
            return LlmErrorHookOutcome::noop();
        };
        if retry_after > MAX_RETRY_AFTER_SECS {
            return LlmErrorHookOutcome::noop();
        }
        let Some(store) = &ctx.services.schedule_store else {
            return LlmErrorHookOutcome::noop();
        };

        let now = chrono::Utc::now();
        let schedules = match store.list_schedules(ctx.session_id).await {
            Ok(schedules) => schedules,
            Err(error) => {
                tracing::warn!(
                    session_id = %ctx.session_id,
                    error = %error,
                    "rate_limit_auto_retry: could not list schedules; not retrying"
                );
                return LlmErrorHookOutcome::noop();
            }
        };
        let own: Vec<&SessionSchedule> = schedules
            .iter()
            .filter(|s| s.description == RATE_LIMIT_RETRY_PROMPT && s.cron_expression.is_none())
            .collect();
        // A retry is already queued (two failures racing): it covers this one.
        if own.iter().any(|s| s.enabled && s.trigger_count == 0) {
            return LlmErrorHookOutcome::noop().with_error_field("auto_retry", true);
        }
        let window_start = now - chrono::Duration::seconds(RETRY_WINDOW_SECS);
        let recent = own.iter().filter(|s| s.created_at > window_start).count();
        if recent >= MAX_RETRIES_PER_WINDOW {
            tracing::info!(
                session_id = %ctx.session_id,
                recent,
                "rate_limit_auto_retry: retry cap reached; leaving the session for a person"
            );
            return LlmErrorHookOutcome::noop();
        }

        let jitter = rand::rng().random_range(1..=MAX_JITTER_SECS);
        let scheduled_at = now + chrono::Duration::seconds((retry_after + jitter) as i64);
        match store
            .create_schedule_enforcing_limits(
                ctx.session_id,
                RATE_LIMIT_RETRY_PROMPT.to_string(),
                None,
                Some(scheduled_at),
                "UTC".to_string(),
            )
            .await
        {
            Ok(schedule) => {
                tracing::info!(
                    session_id = %ctx.session_id,
                    schedule_id = %schedule.id,
                    scheduled_at = %scheduled_at,
                    "rate_limit_auto_retry: scheduled retry after provider rate limit"
                );
                LlmErrorHookOutcome::noop().with_error_field("auto_retry", true)
            }
            Err(_) => {
                tracing::warn!(
                    session_id = %ctx.session_id,
                    "rate_limit_auto_retry: schedule limits refused the retry"
                );
                LlmErrorHookOutcome::noop()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::llm_error_hook::LlmErrorHookServices;
    use crate::engine::typed_id::{PrincipalId, ScheduleId, SessionId};
    use crate::session_services::SessionScheduleStore;
    use chrono::{DateTime, Utc};
    use everruns_contracts::user_facing_error::UserFacingErrorFields;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Store {
        existing: Mutex<Vec<SessionSchedule>>,
        created: Mutex<Vec<(String, Option<DateTime<Utc>>)>>,
        active: u32,
    }

    fn schedule(
        session_id: SessionId,
        description: &str,
        created_at: DateTime<Utc>,
        trigger_count: u32,
    ) -> SessionSchedule {
        SessionSchedule {
            id: ScheduleId::new(),
            session_id,
            owner_principal_id: PrincipalId::new(),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            description: description.to_string(),
            cron_expression: None,
            scheduled_at: Some(created_at),
            timezone: "UTC".to_string(),
            enabled: trigger_count == 0,
            schedule_type: SessionSchedule::derive_type(&None),
            next_trigger_at: None,
            last_triggered_at: None,
            trigger_count,
            created_at,
            updated_at: created_at,
            deleted_at: None,
        }
    }

    #[async_trait]
    impl SessionScheduleStore for Store {
        async fn create_schedule(
            &self,
            session_id: SessionId,
            description: String,
            _cron_expression: Option<String>,
            scheduled_at: Option<DateTime<Utc>>,
            _timezone: String,
        ) -> everruns_contracts::error::Result<SessionSchedule> {
            self.created
                .lock()
                .unwrap()
                .push((description.clone(), scheduled_at));
            Ok(schedule(session_id, &description, Utc::now(), 0))
        }

        async fn cancel_schedule(
            &self,
            _session_id: SessionId,
            _schedule_id: ScheduleId,
        ) -> everruns_contracts::error::Result<SessionSchedule> {
            unimplemented!("not used by these tests")
        }

        async fn list_schedules(
            &self,
            _session_id: SessionId,
        ) -> everruns_contracts::error::Result<Vec<SessionSchedule>> {
            Ok(self.existing.lock().unwrap().clone())
        }

        async fn count_active_schedules(
            &self,
            _session_id: SessionId,
        ) -> everruns_contracts::error::Result<u32> {
            Ok(self.active)
        }

        async fn count_active_org_schedules(&self) -> everruns_contracts::error::Result<u32> {
            Ok(0)
        }
    }

    fn fields(retry_after: u64) -> UserFacingErrorFields {
        let mut fields = UserFacingErrorFields::new();
        fields.insert("retry_after".to_string(), json!(retry_after));
        fields.insert("reason".to_string(), json!("in_flight_budget_exhausted"));
        fields
    }

    async fn run(
        store: &Arc<Store>,
        code: &str,
        fields: &UserFacingErrorFields,
    ) -> LlmErrorHookOutcome {
        let services = LlmErrorHookServices {
            schedule_store: Some(store.clone()),
        };
        let ctx = LlmErrorContext {
            session_id: SessionId::new(),
            error_code: code,
            error_fields: fields,
            config: &Value::Null,
            services: &services,
        };
        RateLimitAutoRetryHook.on_llm_error(&ctx).await
    }

    #[tokio::test]
    async fn schedules_one_retry_after_the_requested_wait() {
        let store = Arc::new(Store::default());
        let before = Utc::now();
        let outcome = run(
            &store,
            user_facing_error_codes::PROVIDER_RATE_LIMITED,
            &fields(120),
        )
        .await;

        assert_eq!(
            outcome.extra_error_fields.get("auto_retry"),
            Some(&json!(true))
        );
        let created = store.created.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].0, RATE_LIMIT_RETRY_PROMPT);
        let at = created[0].1.expect("one-shot fire time");
        assert!(at >= before + chrono::Duration::seconds(121), "{at}");
        assert!(at <= Utc::now() + chrono::Duration::seconds(120 + MAX_JITTER_SECS as i64));
    }

    #[tokio::test]
    async fn ignores_other_codes_long_waits_and_missing_waits() {
        let store = Arc::new(Store::default());
        let quota = run(
            &store,
            user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED,
            &fields(120),
        )
        .await;
        let long = run(
            &store,
            user_facing_error_codes::PROVIDER_RATE_LIMITED,
            &fields(MAX_RETRY_AFTER_SECS + 1),
        )
        .await;
        let none = run(
            &store,
            user_facing_error_codes::PROVIDER_RATE_LIMITED,
            &UserFacingErrorFields::new(),
        )
        .await;
        for outcome in [quota, long, none] {
            assert_eq!(outcome, LlmErrorHookOutcome::noop());
        }
        assert!(store.created.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn caps_retries_per_session_window() {
        let session = SessionId::new();
        let now = Utc::now();
        let store = Arc::new(Store::default());
        {
            let mut existing = store.existing.lock().unwrap();
            for _ in 0..MAX_RETRIES_PER_WINDOW {
                existing.push(schedule(
                    session,
                    RATE_LIMIT_RETRY_PROMPT,
                    now - chrono::Duration::minutes(5),
                    1,
                ));
            }
        }
        let outcome = run(
            &store,
            user_facing_error_codes::PROVIDER_RATE_LIMITED,
            &fields(60),
        )
        .await;
        assert_eq!(outcome, LlmErrorHookOutcome::noop());
        assert!(store.created.lock().unwrap().is_empty());

        // Retries older than the window, and unrelated schedules, do not count.
        {
            let mut existing = store.existing.lock().unwrap();
            existing.clear();
            for _ in 0..MAX_RETRIES_PER_WINDOW {
                existing.push(schedule(
                    session,
                    RATE_LIMIT_RETRY_PROMPT,
                    now - chrono::Duration::seconds(RETRY_WINDOW_SECS + 60),
                    1,
                ));
                existing.push(schedule(session, "Daily report", now, 1));
            }
        }
        run(
            &store,
            user_facing_error_codes::PROVIDER_RATE_LIMITED,
            &fields(60),
        )
        .await;
        assert_eq!(store.created.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_queued_retry_is_not_duplicated() {
        let store = Arc::new(Store::default());
        store.existing.lock().unwrap().push(schedule(
            SessionId::new(),
            RATE_LIMIT_RETRY_PROMPT,
            Utc::now(),
            0,
        ));
        let outcome = run(
            &store,
            user_facing_error_codes::PROVIDER_RATE_LIMITED,
            &fields(60),
        )
        .await;
        assert_eq!(
            outcome.extra_error_fields.get("auto_retry"),
            Some(&json!(true))
        );
        assert!(store.created.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn schedule_limits_suppress_the_promise() {
        let store = Arc::new(Store {
            active: crate::session_schedule::MAX_ACTIVE_SCHEDULES_PER_SESSION,
            ..Default::default()
        });
        let outcome = run(
            &store,
            user_facing_error_codes::PROVIDER_RATE_LIMITED,
            &fields(60),
        )
        .await;
        assert_eq!(outcome, LlmErrorHookOutcome::noop());
    }
}
