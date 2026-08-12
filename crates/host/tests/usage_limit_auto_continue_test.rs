//! End-to-end runtime test for the `usage_limit_auto_continue` capability.
//!
//! Drives a real turn through the in-process runtime with an `LlmSim` driver
//! scripted to fail with a Codex-style `usage_limit_reached` body, and verifies
//! the `LlmErrorHook` seam fires end-to-end: a continuation is scheduled through
//! the injected schedule store, and the user-facing error copy promises
//! automatic resumption. A contrast case (capability disabled) proves the
//! behavior is fully gated by the capability — the copy stays generic and
//! nothing is scheduled even though the schedule store is present.

use async_trait::async_trait;
use everruns_builtins::UsageLimitAutoContinueCapability;
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::session_schedule::SessionSchedule;
use everruns_core::traits::SessionScheduleStore;
use everruns_core::typed_id::{PrincipalId, ScheduleId, SessionId};
use everruns_core::{
    AgentId, CapabilityRegistry, DriverId, HarnessId, PlatformDefinition, ResolvedModel,
};
use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::llmsim_driver::{LlmSimConfig, SimError, SimTurn};
use std::sync::{Arc, Mutex};

/// Codex/ChatGPT usage-limit 429 body. Classifies to
/// `provider_usage_limit_reached` and carries an absolute `resets_at`.
const CODEX_USAGE_LIMIT_BODY: &str = r#"Codex API error (429 Too Many Requests): {"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","plan_type":"pro","resets_at":1783767823,"eligible_promo":null,"resets_in_seconds":12337}}"#;

/// Records the one-shot schedules created during a turn.
#[derive(Default)]
struct RecordingScheduleStore {
    created: Mutex<Vec<(String, Option<chrono::DateTime<chrono::Utc>>)>>,
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
    ) -> everruns_core::Result<SessionSchedule> {
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
    ) -> everruns_core::Result<SessionSchedule> {
        unimplemented!("not used by this test")
    }

    async fn list_schedules(
        &self,
        _session_id: SessionId,
    ) -> everruns_core::Result<Vec<SessionSchedule>> {
        Ok(vec![])
    }

    async fn count_active_schedules(&self, _session_id: SessionId) -> everruns_core::Result<u32> {
        Ok(0)
    }

    async fn count_active_org_schedules(&self) -> everruns_core::Result<u32> {
        Ok(0)
    }
}

fn platform_with_capability() -> PlatformDefinition {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(UsageLimitAutoContinueCapability);
    PlatformDefinition::new(capabilities, DriverRegistry::new())
}

fn llmsim_model() -> ResolvedModel {
    ResolvedModel {
        model: "llmsim-model".into(),
        provider_type: DriverId::LlmSim,
        api_key: Some("fake-key".into()),
        base_url: None,
        provider_metadata: None,
    }
}

/// Collect every message text in the session transcript.
macro_rules! message_texts {
    ($runtime:expr, $session_id:expr) => {
        $runtime
            .messages($session_id)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|m| m.text().map(|t| t.to_string()))
            .collect::<Vec<String>>()
    };
}

#[tokio::test]
async fn usage_limit_error_schedules_continuation_and_promises_resume() {
    let harness_id: HarnessId = "harness_00000000000000000000000000000091".parse().unwrap();
    let agent_id: AgentId = "agent_00000000000000000000000000000091".parse().unwrap();
    let session_id: SessionId = "session_00000000000000000000000000000091".parse().unwrap();

    let store = Arc::new(RecordingScheduleStore::default());
    let store_for_factory = store.clone();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform_with_capability())
        .llm_sim(LlmSimConfig::scripted(vec![SimTurn::Error(
            SimError::Other(CODEX_USAGE_LIMIT_BODY.to_string()),
        )]))
        .default_model(llmsim_model())
        .with_schedule_store_factory(Arc::new(move |_org_id| {
            store_for_factory.clone() as Arc<dyn SessionScheduleStore>
        }))
        .harness(
            HarnessBuilder::new("limits", "You do long-running work.")
                .id(harness_id)
                .capability("usage_limit_auto_continue")
                .build(),
        )
        .agent(
            AgentBuilder::new("worker", "Do the work.")
                .id(agent_id)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .build(),
        )
        .build()
        .await
        .unwrap();

    // The turn fails on the usage limit; we assert on the persisted effects
    // regardless of the returned success flag.
    let _ = runtime.run_text_turn(session_id, "Do the long task.").await;

    // A continuation was scheduled through the injected store.
    {
        let created = store.created.lock().unwrap();
        assert_eq!(
            created.len(),
            1,
            "exactly one continuation must be scheduled"
        );
        assert_eq!(created[0].0, "Continue tasks");
        assert!(created[0].1.is_some(), "continuation must have a fire time");
    }

    // The user-facing error copy names the limit and promises resumption.
    let texts = message_texts!(runtime, session_id);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("You're out of LLM usage limits")
                && t.contains("continue work automatically")),
        "error copy should promise auto-continue, got: {texts:?}"
    );
}

#[tokio::test]
async fn usage_limit_error_without_capability_stays_generic() {
    let harness_id: HarnessId = "harness_00000000000000000000000000000092".parse().unwrap();
    let agent_id: AgentId = "agent_00000000000000000000000000000092".parse().unwrap();
    let session_id: SessionId = "session_00000000000000000000000000000092".parse().unwrap();

    let store = Arc::new(RecordingScheduleStore::default());
    let store_for_factory = store.clone();

    // Same failure and schedule store, but the capability is NOT enabled on the
    // harness. Nothing should be scheduled and the copy must stay generic.
    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform_with_capability())
        .llm_sim(LlmSimConfig::scripted(vec![SimTurn::Error(
            SimError::Other(CODEX_USAGE_LIMIT_BODY.to_string()),
        )]))
        .default_model(llmsim_model())
        .with_schedule_store_factory(Arc::new(move |_org_id| {
            store_for_factory.clone() as Arc<dyn SessionScheduleStore>
        }))
        .harness(
            HarnessBuilder::new("limits", "You do long-running work.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("worker", "Do the work.")
                .id(agent_id)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .build(),
        )
        .build()
        .await
        .unwrap();

    let _ = runtime.run_text_turn(session_id, "Do the long task.").await;

    assert!(
        store.created.lock().unwrap().is_empty(),
        "no continuation should be scheduled without the capability"
    );

    let texts = message_texts!(runtime, session_id);
    // Still classified as a usage limit (core behavior), but no auto-continue promise.
    assert!(
        texts
            .iter()
            .any(|t| t.contains("You're out of LLM usage limits")),
        "error should still classify as a usage limit, got: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .all(|t| !t.contains("continue work automatically")),
        "generic copy must not promise auto-continue, got: {texts:?}"
    );
}
