// Usage-Limit Auto-Continue capability
//
// When an LLM subscription/plan usage limit is hit — classified as
// `provider_usage_limit_reached` with a concrete `resets_at` timestamp (e.g. the
// ChatGPT/Codex `usage_limit_reached` body) — this capability makes the session
// resume on its own once the limit resets.
//
// The behavior is fully encapsulated behind the platform's `LlmErrorHook`
// seam (`crate::llm_error_hook`): the capability contributes no tools and no
// reason-atom special-casing. It supplies an error hook that, on the terminal
// error path, when the error code matches:
//
//   1. schedules a one-shot session continuation at `resets_at + delay_seconds`
//      that re-injects `prompt` to resume the interrupted work, and
//   2. returns an `auto_continue` error field so the user-facing copy promises
//      automatic resumption.
//
// (2) is returned only when (1) actually succeeded, so the copy never
// over-promises. The reason atom invokes this hook generically alongside any
// other capability's hook — new error-recovery extensions are built the same
// way, without touching the atom.

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};
use crate::capability_types::AgentCapabilityConfig;
use crate::llm_error_hook::{LlmErrorContext, LlmErrorHook, LlmErrorHookOutcome};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub const USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID: &str = "usage_limit_auto_continue";

/// Default delay, in seconds, after the reported reset time before the
/// continuation fires. The reset is a lower bound; a small buffer avoids racing
/// the provider's own clock (the motivating Codex case wants reset + 2 min).
pub const DEFAULT_CONTINUATION_DELAY_SECS: i64 = 120;

/// Default prompt re-injected to resume work once the limit resets.
pub const DEFAULT_CONTINUATION_PROMPT: &str = "Continue tasks";

/// Upper bound on the configurable delay (24h) so a misconfiguration cannot park
/// a continuation arbitrarily far in the future.
const MAX_CONTINUATION_DELAY_SECS: i64 = 86_400;

/// Resolved auto-continue settings for a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoContinueConfig {
    /// Seconds to wait after `resets_at` before firing the continuation.
    pub delay_seconds: i64,
    /// Prompt injected as the continuation turn's user message.
    pub prompt: String,
}

impl Default for AutoContinueConfig {
    fn default() -> Self {
        Self {
            delay_seconds: DEFAULT_CONTINUATION_DELAY_SECS,
            prompt: DEFAULT_CONTINUATION_PROMPT.to_string(),
        }
    }
}

impl AutoContinueConfig {
    /// Parse settings from a single capability config JSON value, falling back
    /// to defaults for any missing, blank, or out-of-range field.
    pub fn from_config_value(config: &Value) -> Self {
        let delay_seconds = config
            .get("delay_seconds")
            .and_then(Value::as_i64)
            .filter(|secs| (0..=MAX_CONTINUATION_DELAY_SECS).contains(secs))
            .unwrap_or(DEFAULT_CONTINUATION_DELAY_SECS);

        let prompt = config
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(DEFAULT_CONTINUATION_PROMPT)
            .to_string();

        Self {
            delay_seconds,
            prompt,
        }
    }
}

/// Resolve the auto-continue settings for a turn, or `None` when the capability
/// is not enabled for the agent/session.
pub fn resolve_usage_limit_auto_continue(
    configs: &[AgentCapabilityConfig],
) -> Option<AutoContinueConfig> {
    configs
        .iter()
        .find(|config| config.capability_id() == USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID)
        .map(|config| AutoContinueConfig::from_config_value(config.config_value()))
}

pub struct UsageLimitAutoContinueCapability;

impl Capability for UsageLimitAutoContinueCapability {
    fn id(&self) -> &str {
        USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Auto-Continue After Usage Limit"
    }

    fn description(&self) -> &str {
        "When an LLM usage limit is reached, automatically resume the interrupted work shortly after the limit resets."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Автопродовження після ліміту використання",
            "Коли досягнуто ліміт використання LLM, автоматично відновлює перервану роботу невдовзі після скидання ліміту.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Low
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn llm_error_hook(&self) -> Option<Arc<dyn LlmErrorHook>> {
        Some(Arc::new(UsageLimitAutoContinueHook))
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "delay_seconds": {
                    "type": "integer",
                    "title": "Continuation delay (seconds)",
                    "description": "How long to wait after the reported reset time before resuming work. A small buffer avoids racing the provider's reset clock.",
                    "minimum": 0,
                    "maximum": MAX_CONTINUATION_DELAY_SECS,
                    "default": DEFAULT_CONTINUATION_DELAY_SECS
                },
                "prompt": {
                    "type": "string",
                    "title": "Continuation prompt",
                    "description": "Message injected to resume the interrupted work when the limit resets.",
                    "default": DEFAULT_CONTINUATION_PROMPT
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if let Some(delay) = config.get("delay_seconds") {
            let secs = delay
                .as_i64()
                .ok_or_else(|| "delay_seconds must be an integer".to_string())?;
            if !(0..=MAX_CONTINUATION_DELAY_SECS).contains(&secs) {
                return Err(format!(
                    "delay_seconds must be between 0 and {MAX_CONTINUATION_DELAY_SECS}"
                ));
            }
        }
        if let Some(prompt) = config.get("prompt")
            && !prompt.is_string()
        {
            return Err("prompt must be a string".to_string());
        }
        Ok(())
    }
}

/// Error hook that encapsulates the auto-continue behavior. Invoked by the
/// reason atom through the generic `LlmErrorHook` seam on a terminal LLM
/// error; the reason atom itself has no knowledge of usage limits or scheduling.
struct UsageLimitAutoContinueHook;

#[async_trait]
impl LlmErrorHook for UsageLimitAutoContinueHook {
    async fn on_llm_error(&self, ctx: &LlmErrorContext<'_>) -> LlmErrorHookOutcome {
        // Only act on the usage-limit error this capability recovers from.
        if ctx.error_code != crate::user_facing_error_codes::PROVIDER_USAGE_LIMIT_REACHED {
            return LlmErrorHookOutcome::noop();
        }
        // Best-effort: without a schedule store or a concrete reset time there
        // is nothing to schedule, so make no auto-resume promise.
        let Some(store) = &ctx.services.schedule_store else {
            return LlmErrorHookOutcome::noop();
        };
        let Some(resets_at) = ctx.error_fields.get("resets_at").and_then(|v| v.as_i64()) else {
            return LlmErrorHookOutcome::noop();
        };
        let Some(reset_time) = chrono::DateTime::from_timestamp(resets_at, 0) else {
            return LlmErrorHookOutcome::noop();
        };

        let cfg = AutoContinueConfig::from_config_value(ctx.config);

        // Fire `delay_seconds` after the reset. Clamp into the future so a
        // stale/past reset (clock skew, limit already cleared) still fires
        // instead of being dropped by the store's past-time next-trigger rule.
        let now = chrono::Utc::now();
        let delay = chrono::Duration::seconds(cfg.delay_seconds);
        let scheduled_at = (reset_time + delay).max(now + delay);

        match store
            .create_schedule_enforcing_limits(
                ctx.session_id,
                cfg.prompt.clone(),
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
                    "usage_limit_auto_continue: scheduled continuation after usage-limit reset"
                );
                // Unlock the "we'll continue automatically" message copy only
                // now that a continuation was actually scheduled.
                LlmErrorHookOutcome::noop().with_error_field("auto_continue", true)
            }
            Err(_) => {
                tracing::warn!(
                    session_id = %ctx.session_id,
                    "usage_limit_auto_continue: failed to schedule continuation within schedule limits"
                );
                LlmErrorHookOutcome::noop()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap_config(config: Value) -> AgentCapabilityConfig {
        AgentCapabilityConfig::with_config(USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID, config)
    }

    #[test]
    fn resolve_returns_none_without_capability() {
        assert_eq!(resolve_usage_limit_auto_continue(&[]), None);
    }

    #[test]
    fn resolve_uses_defaults_for_empty_config() {
        let resolved = resolve_usage_limit_auto_continue(&[cap_config(json!({}))]).unwrap();
        assert_eq!(resolved, AutoContinueConfig::default());
    }

    #[test]
    fn resolve_reads_custom_delay_and_prompt() {
        let resolved = resolve_usage_limit_auto_continue(&[cap_config(json!({
            "delay_seconds": 300,
            "prompt": "  Resume the migration  "
        }))])
        .unwrap();
        assert_eq!(resolved.delay_seconds, 300);
        assert_eq!(resolved.prompt, "Resume the migration");
    }

    #[test]
    fn resolve_falls_back_on_out_of_range_or_blank_fields() {
        let resolved = resolve_usage_limit_auto_continue(&[cap_config(json!({
            "delay_seconds": -5,
            "prompt": "   "
        }))])
        .unwrap();
        assert_eq!(resolved.delay_seconds, DEFAULT_CONTINUATION_DELAY_SECS);
        assert_eq!(resolved.prompt, DEFAULT_CONTINUATION_PROMPT);
    }

    #[test]
    fn validate_rejects_bad_types_and_ranges() {
        let cap = UsageLimitAutoContinueCapability;
        assert!(
            cap.validate_config(&json!({ "delay_seconds": 120 }))
                .is_ok()
        );
        assert!(
            cap.validate_config(&json!({ "delay_seconds": "x" }))
                .is_err()
        );
        assert!(
            cap.validate_config(&json!({ "delay_seconds": 999999999 }))
                .is_err()
        );
        assert!(cap.validate_config(&json!({ "prompt": 5 })).is_err());
    }

    // ------------------------------------------------------------------------
    // Handler tests (the encapsulated behavior, exercised via the seam)
    // ------------------------------------------------------------------------

    use crate::llm_error_hook::{LlmErrorContext, LlmErrorHook, LlmErrorHookServices};
    use crate::session_schedule::SessionSchedule;
    use crate::typed_id::{PrincipalId, ScheduleId, SessionId};
    use crate::user_facing_error::UserFacingErrorFields;
    use crate::user_facing_error_codes;
    use everruns_core::session_services::SessionScheduleStore;
    use std::sync::Mutex;

    /// Records the one-shot schedules the handler creates so tests can assert on
    /// the description and fire time.
    #[derive(Default)]
    struct RecordingScheduleStore {
        created: Mutex<Vec<(String, Option<chrono::DateTime<chrono::Utc>>)>>,
        active_schedules: u32,
    }

    #[async_trait]
    impl SessionScheduleStore for RecordingScheduleStore {
        async fn create_schedule(
            &self,
            session_id: SessionId,
            description: String,
            cron_expression: Option<String>,
            scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
            timezone: String,
        ) -> crate::error::Result<SessionSchedule> {
            self.created
                .lock()
                .unwrap()
                .push((description.clone(), scheduled_at));
            Ok(SessionSchedule {
                id: ScheduleId::new(),
                session_id,
                owner_principal_id: PrincipalId::new(),
                resolved_owner_user_id: None,
                owner: None,
                effective_owner: None,
                description,
                cron_expression: cron_expression.clone(),
                scheduled_at,
                timezone,
                enabled: true,
                schedule_type: SessionSchedule::derive_type(&cron_expression),
                next_trigger_at: scheduled_at,
                last_triggered_at: None,
                trigger_count: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn cancel_schedule(
            &self,
            _session_id: SessionId,
            _schedule_id: ScheduleId,
        ) -> crate::error::Result<SessionSchedule> {
            unimplemented!("not used by these tests")
        }

        async fn list_schedules(
            &self,
            _session_id: SessionId,
        ) -> crate::error::Result<Vec<SessionSchedule>> {
            Ok(vec![])
        }

        async fn count_active_schedules(
            &self,
            _session_id: SessionId,
        ) -> crate::error::Result<u32> {
            Ok(self.active_schedules)
        }

        async fn count_active_org_schedules(&self) -> crate::error::Result<u32> {
            Ok(0)
        }
    }

    fn usage_limit_fields(resets_at: i64) -> UserFacingErrorFields {
        let mut fields = UserFacingErrorFields::new();
        fields.insert("resets_at".to_string(), json!(resets_at));
        fields
    }

    #[tokio::test]
    async fn handler_schedules_continuation_and_flags_auto_continue() {
        let store = Arc::new(RecordingScheduleStore::default());
        let services = LlmErrorHookServices {
            schedule_store: Some(store.clone()),
        };
        let fields = usage_limit_fields(1_783_767_823);
        let config = json!({ "prompt": "Resume the migration" });
        let ctx = LlmErrorContext {
            session_id: SessionId::new(),
            error_code: user_facing_error_codes::PROVIDER_USAGE_LIMIT_REACHED,
            error_fields: &fields,
            config: &config,
            services: &services,
        };

        let outcome = UsageLimitAutoContinueHook.on_llm_error(&ctx).await;

        // Continuation scheduled with the configured prompt.
        let created = store.created.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].0, "Resume the migration");
        assert!(created[0].1.is_some(), "continuation must have a fire time");

        // Only now is the auto-continue message copy unlocked.
        assert_eq!(
            outcome.extra_error_fields.get("auto_continue"),
            Some(&json!(true))
        );
    }

    #[tokio::test]
    async fn handler_enforces_schedule_limits_before_auto_continue() {
        let store = Arc::new(RecordingScheduleStore {
            active_schedules: crate::session_schedule::MAX_ACTIVE_SCHEDULES_PER_SESSION,
            ..Default::default()
        });
        let services = LlmErrorHookServices {
            schedule_store: Some(store.clone()),
        };
        let fields = usage_limit_fields(1_783_767_823);
        let config = json!({});
        let ctx = LlmErrorContext {
            session_id: SessionId::new(),
            error_code: user_facing_error_codes::PROVIDER_USAGE_LIMIT_REACHED,
            error_fields: &fields,
            config: &config,
            services: &services,
        };

        let outcome = UsageLimitAutoContinueHook.on_llm_error(&ctx).await;

        assert!(store.created.lock().unwrap().is_empty());
        assert_eq!(outcome, LlmErrorHookOutcome::noop());
    }

    #[tokio::test]
    async fn handler_ignores_non_usage_limit_errors() {
        let store = Arc::new(RecordingScheduleStore::default());
        let services = LlmErrorHookServices {
            schedule_store: Some(store.clone()),
        };
        let fields = usage_limit_fields(1_783_767_823);
        let config = json!({});
        let ctx = LlmErrorContext {
            session_id: SessionId::new(),
            error_code: user_facing_error_codes::PROVIDER_RATE_LIMITED,
            error_fields: &fields,
            config: &config,
            services: &services,
        };

        let outcome = UsageLimitAutoContinueHook.on_llm_error(&ctx).await;

        assert!(store.created.lock().unwrap().is_empty());
        assert_eq!(outcome, LlmErrorHookOutcome::noop());
    }

    #[tokio::test]
    async fn handler_makes_no_promise_without_schedule_store() {
        let services = LlmErrorHookServices {
            schedule_store: None,
        };
        let fields = usage_limit_fields(1_783_767_823);
        let config = json!({});
        let ctx = LlmErrorContext {
            session_id: SessionId::new(),
            error_code: user_facing_error_codes::PROVIDER_USAGE_LIMIT_REACHED,
            error_fields: &fields,
            config: &config,
            services: &services,
        };

        let outcome = UsageLimitAutoContinueHook.on_llm_error(&ctx).await;
        assert_eq!(outcome, LlmErrorHookOutcome::noop());
    }
}
