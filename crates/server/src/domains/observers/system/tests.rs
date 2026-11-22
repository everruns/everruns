#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::domains::evals::graders::Graders;
use crate::domains::observers::{
    ObserverWorkerDeps,
    judge::{JudgeClient, JudgeResult, MonitoringAdmission, OrgCompletion, TurnEvidence},
};
use crate::storage::{CreateEventRow, CreateSessionRow, ListTraceScoresParams, StorageBackend};
use everruns_contracts::typed_id::{ModelId, PrincipalId, SessionId};
use everruns_core::{Event, EventContext, events::deserialize_event_data};
use serde_json::{Value, json};
use std::sync::Arc;

async fn fixture(policy: QualityMonitoringPolicy) -> (Arc<StorageBackend>, SessionId) {
    let db = Arc::new(StorageBackend::test_database());
    configure(&db, 1, &policy).await.unwrap();
    let session = db
        .create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    (db, session)
}

async fn event(db: &Arc<StorageBackend>, session: SessionId, kind: &str, data: Value) -> Event {
    let row = db
        .create_event(CreateEventRow {
            session_id: session,
            event_type: kind.into(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: data.clone(),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    let mut event = Event::new(
        session,
        EventContext {
            turn_id: Some(everruns_contracts::typed_id::TurnId::from_uuid(
                session.uuid(),
            )),
            ..Default::default()
        },
        deserialize_event_data(kind, data),
    );
    event.id = row.id;
    event.sequence = Some(row.sequence);
    event.ts = row.ts;
    event
}

async fn compacted(db: &Arc<StorageBackend>, session: SessionId, allowed: bool) -> Event {
    event(db, session, "input.message", json!({"message":{"id":everruns_contracts::typed_id::MessageId::new(),"role":"user","content":[{"type":"text","text":"The deployment deadline is Friday."}],"created_at":chrono::Utc::now()}})).await;
    let event = event(db, session, "context.compacted", json!({"strategy_used":"native","checkpoint_id":uuid::Uuid::now_v7(),"source_sequence":1,"messages_before":20,"messages_after":20,"tokens_before":9000,"tokens_after":2000,"duration_ms":3,"steps":[],"model":"test","history_evidence_allowed":allowed})).await;
    enqueue_event(db, &event).await.unwrap();
    event
}

async fn finish_turn(db: &Arc<StorageBackend>, session: SessionId) {
    event(db, session, "output.message.completed", json!({"message":{"id":everruns_contracts::typed_id::MessageId::new(),"role":"agent","content":[{"type":"text","text":"I will keep the Friday deadline."}],"created_at":chrono::Utc::now()}})).await;
    let turn = event(
        db,
        session,
        "turn.completed",
        json!({"turn_id":everruns_contracts::typed_id::TurnId::from_uuid(session.uuid()),"iterations":1}),
    )
    .await;
    enqueue_event(db, &turn).await.unwrap();
}

async fn drain(db: &Arc<StorageBackend>, graders: Graders) {
    let deps = ObserverWorkerDeps {
        db: db.clone(),
        graders,
    };
    for score in db.claim_trace_scores(32, 120, 3).await.unwrap() {
        let observer = db.get_observer(score.observer_id).await.unwrap().unwrap();
        process_score(&deps, &score, &observer).await.unwrap();
    }
}

async fn scores(db: &StorageBackend) -> Vec<crate::storage::TraceScoreRow> {
    let observer = db.system_compaction_observer(1).await.unwrap().unwrap();
    db.list_trace_scores(
        1,
        ListTraceScoresParams {
            observer_id: observer.id,
            limit: 50,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

struct FakeJudge {
    db: Arc<StorageBackend>,
}
#[async_trait::async_trait]
impl JudgeClient for FakeJudge {
    async fn judge(
        &self,
        _: i64,
        _: Option<ModelId>,
        _: &str,
        _: &TurnEvidence,
    ) -> anyhow::Result<Option<JudgeResult>> {
        Ok(None)
    }
    async fn judge_compaction(
        &self,
        org: i64,
        _: ModelId,
        _: String,
        user: String,
        admission: MonitoringAdmission<'_>,
    ) -> anyhow::Result<Option<OrgCompletion>> {
        if !self
            .db
            .reserve_monitoring_budget(
                org,
                admission.score_id,
                admission.attempt,
                &serde_json::to_value(admission.policy)?,
                1000,
            )
            .await?
        {
            return Err(crate::domains::observers::judge::MonitoringNotAdmitted.into());
        }
        let evidence: evidence::ContinuityEvidence = serde_json::from_str(&user)?;
        let old = evidence
            .excerpts
            .iter()
            .find(|e| e.sequence <= evidence.source_boundary)
            .unwrap();
        let new = evidence
            .excerpts
            .iter()
            .find(|e| e.sequence > evidence.compaction_sequence)
            .unwrap();
        Ok(Some(OrgCompletion { text: json!({"verdict":"passed","confidence":0.95,"evidence_ids":[old.id,new.id],"reason":"The sampled source deadline is retained in the continuation."}).to_string(), model:"fixture".into(), input_tokens:Some(100),output_tokens:Some(20),cost_usd:Some(0.0001) }))
    }
}

fn paid() -> QualityMonitoringPolicy {
    QualityMonitoringPolicy {
        enabled: true,
        model_id: Some(ModelId::new()),
        sampling_rate: 1.0,
        daily_budget_usd: 0.01,
        ..Default::default()
    }
}

#[tokio::test]
async fn opt_in_is_required_and_redelivery_deduplicates_checkpoint_scores() {
    let (db, session) = fixture(Default::default()).await;
    let checkpoint = compacted(&db, session, true).await;
    assert!(scores(&db).await.is_empty());
    configure(
        &db,
        1,
        &QualityMonitoringPolicy {
            enabled: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        enqueue_event(&db, &checkpoint).await.unwrap(),
        0,
        "consent does not backfill old events"
    );
    let checkpoint = compacted(&db, session, true).await;
    assert_eq!(enqueue_event(&db, &checkpoint).await.unwrap(), 0);
    finish_turn(&db, session).await;
    drain(&db, Graders::new(db.clone())).await;
    let rows = scores(&db).await;
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|row| row.scorer_key == EXECUTION_KEY && row.pass == Some(true))
    );
    assert!(
        rows.iter()
            .any(|row| row.scorer_key == CONTINUITY_KEY && row.pass.is_none())
    );
    assert_eq!(db.monitoring_spend(1).await.unwrap().reserved_usd, 0.0);
    let caller = everruns_core::Caller {
        org_id: 1,
        org_public_id: "org_00000000000000000000000000000001".into(),
        user_id: Some(uuid::Uuid::now_v7()),
        role: everruns_core::OrgRole::Admin,
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    };
    let ctx = crate::domains::common::Ctx::minimal_for_test(caller, db.clone(), None);
    let reviewed = crate::domains::common::dispatch(
        "review_org_quality_monitoring_score",
        json!({"org":ctx.caller.org_public_id,"score":rows[0].public_id,"agrees":false}),
        &ctx,
    )
    .await
    .unwrap();
    let reviewed: Value = serde_json::from_str(&reviewed).unwrap();
    assert_eq!(reviewed["review"]["agrees"], false);
    assert_eq!(db.monitoring_spend(1).await.unwrap().reserved_usd, 0.0);
}

#[tokio::test]
async fn admitted_continuity_uses_source_and_post_checkpoint_evidence() {
    let (db, session) = fixture(paid()).await;
    compacted(&db, session, true).await;
    finish_turn(&db, session).await;
    let graders = Graders::new(db.clone()).with_judge(Arc::new(FakeJudge { db: db.clone() }));
    drain(&db, graders).await;
    let rows = scores(&db).await;
    assert_eq!(rows.iter().filter(|row| row.pass == Some(true)).count(), 2);
    assert!(
        rows.iter()
            .find(|row| row.scorer_key == CONTINUITY_KEY)
            .unwrap()
            .reason
            .as_ref()
            .unwrap()
            .contains("event_")
    );
    assert_eq!(db.monitoring_spend(1).await.unwrap().reserved_usd, 0.001);
}

#[tokio::test]
async fn revocation_and_runtime_history_permission_prevent_paid_assessment() {
    for revoke in [false, true] {
        let (db, session) = fixture(paid()).await;
        compacted(&db, session, revoke).await;
        finish_turn(&db, session).await;
        if revoke {
            configure(&db, 1, &QualityMonitoringPolicy::default())
                .await
                .unwrap();
        }
        drain(
            &db,
            Graders::new(db.clone()).with_judge(Arc::new(FakeJudge { db: db.clone() })),
        )
        .await;
        assert!(
            scores(&db)
                .await
                .iter()
                .filter(|row| row.scorer_key == CONTINUITY_KEY)
                .all(|row| row.pass.is_none())
        );
        assert_eq!(db.monitoring_spend(1).await.unwrap().reserved_usd, 0.0);
    }
}

#[tokio::test]
async fn budget_reservations_are_atomic_and_cannot_be_reused_by_duplicate_attempts() {
    let p = paid();
    let (db, session) = fixture(p.clone()).await;
    for _ in 0..12 {
        compacted(&db, session, true).await;
    }
    let claimed = db.claim_trace_scores(32, 120, 3).await.unwrap();
    let policy = Arc::new(serde_json::to_value(&p).unwrap());
    let futures = claimed.iter().take(12).map(|score| {
        let db = db.clone();
        let policy = policy.clone();
        let id = score.id;
        async move {
            db.reserve_monitoring_budget(1, id, 1, &policy, 3000)
                .await
                .unwrap()
                .then_some(id)
        }
    });
    let admitted: Vec<_> = futures::future::join_all(futures)
        .await
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(admitted.len(), 3);
    assert_eq!(db.monitoring_spend(1).await.unwrap().reserved_usd, 0.009);
    assert!(
        !db.reserve_monitoring_budget(1, admitted[0], 1, &policy, 1)
            .await
            .unwrap()
    );
    assert!(
        !db.reserve_monitoring_budget(2, claimed[0].id, 1, &policy, 1)
            .await
            .unwrap()
    );
}

#[test]
fn fabricated_citations_and_unsupported_confidence_cannot_create_a_pass() {
    use everruns_core::compaction_observation::CompactionEvidenceExcerpt as E;
    let evidence = evidence::ContinuityEvidence {
        source_boundary: 10,
        compaction_sequence: 20,
        continuation_sequence: 30,
        excerpts: vec![
            E {
                id: "old".into(),
                sequence: 1,
                event_type: "input.message".into(),
                text: "Friday".into(),
                truncated: false,
            },
            E {
                id: "new".into(),
                sequence: 25,
                event_type: "output.message.completed".into(),
                text: "Friday".into(),
                truncated: false,
            },
        ],
    };
    assert!(
        evidence::parse(
            r#"{"verdict":"passed","confidence":1,"evidence_ids":["fake","new"],"reason":"ok"}"#,
            &evidence
        )
        .is_err()
    );
    assert_eq!(
        evidence::parse(
            r#"{"verdict":"passed","confidence":0.5,"evidence_ids":["old","new"],"reason":"ok"}"#,
            &evidence
        )
        .unwrap()
        .verdict,
        everruns_core::compaction_observation::CompactionVerdict::InsufficientEvidence
    );
}

#[tokio::test]
async fn excluded_and_eval_sessions_do_not_enqueue_content_checks() {
    for tag in ["private", "eval"] {
        let mut policy = paid();
        policy.excluded_tags = vec!["private".into()];
        let (db, _) = fixture(policy).await;
        let session = db
            .create_session(CreateSessionRow {
                org_id: 1,
                owner_principal_id: PrincipalId::from_seed(1),
                tags: vec![tag.into()],
                ..Default::default()
            })
            .await
            .unwrap()
            .id;
        compacted(&db, session, true).await;
        assert!(scores(&db).await.is_empty());
        assert_eq!(db.monitoring_spend(1).await.unwrap().reserved_usd, 0.0);
    }
}

#[tokio::test]
async fn disabling_does_not_require_the_selected_model_to_still_exist() {
    let db = StorageBackend::test_database();
    let mut policy = paid();
    assert!(validate_policy(&db, 1, &policy).await.is_err());
    policy.enabled = false;
    validate_policy(&db, 1, &policy).await.unwrap();
}

#[tokio::test]
async fn stale_attempt_cannot_finish_or_reserve_and_previews_are_bounded() {
    let (db, session) = fixture(paid()).await;
    compacted(&db, session, true).await;
    let claimed = db.claim_trace_scores(32, 120, 3).await.unwrap();
    let score = claimed
        .iter()
        .find(|row| row.scorer_key == EXECUTION_KEY)
        .unwrap();
    let mut stale = score.clone();
    stale.attempts += 1;
    assert!(
        !db.finish_monitoring_score(&stale, Default::default())
            .await
            .unwrap()
    );
    let policy = db.org_quality_monitoring(1).await.unwrap().unwrap();
    assert!(
        !db.reserve_monitoring_budget(1, score.id, stale.attempts, &policy, 1)
            .await
            .unwrap()
    );
    event(
        &db,
        session,
        "tool.completed",
        json!({"result":"世🙂".repeat(10000)}),
    )
    .await;
    assert!(db.monitoring_previews(session, 1, 65, 10).await.is_err());
    let previews = db.monitoring_previews(session, 1, 32, 17).await.unwrap();
    assert!(previews.iter().all(|e| e.text.chars().count() <= 17));
    assert!(previews.iter().any(|e| e.truncated));
}

#[tokio::test]
async fn organization_reports_and_review_are_tenant_and_permission_scoped() {
    use crate::domains::common::{Ctx, dispatch};
    use everruns_core::{Caller, OrgRole};
    let db = Arc::new(StorageBackend::test_database());
    let org = "org_00000000000000000000000000000001";
    let caller = |role| Caller {
        org_id: 1,
        org_public_id: org.into(),
        user_id: Some(uuid::Uuid::now_v7()),
        role,
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    };
    let member = Ctx::minimal_for_test(caller(OrgRole::Member), db.clone(), None);
    let admin = Ctx::minimal_for_test(caller(OrgRole::Admin), db.clone(), None);
    let state = dispatch("get_org_quality_monitoring", json!({"org":org}), &admin)
        .await
        .unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["policy"]["enabled"], false);
    assert_eq!(state["today"]["reserved_usd"].as_f64(), Some(0.0));
    assert!(
        dispatch(
            "get_org_quality_monitoring",
            json!({"org":"org_foreign"}),
            &admin
        )
        .await
        .is_err()
    );
    struct WithoutSessionAccess;
    impl everruns_core::PermissionResolver for WithoutSessionAccess {
        fn has_permission(&self, _: &Caller, permission: &everruns_core::Permission) -> bool {
            permission != &everruns_core::Permission::OrgSessionsManage
        }
        fn caller_permissions(&self, caller: &Caller) -> Vec<everruns_core::Permission> {
            everruns_core::Permission::ALL
                .iter()
                .copied()
                .filter(|p| self.has_permission(caller, p))
                .collect()
        }
    }
    let mut restricted = Ctx::minimal_for_test(caller(OrgRole::Admin), db.clone(), None);
    restricted.permission_resolver = Arc::new(WithoutSessionAccess);
    let denied = dispatch(
        "review_org_quality_monitoring_score",
        json!({"org":org,"score":"score_foreign","agrees":true}),
        &restricted,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        denied.kind,
        crate::domains::common::CommandErrorKind::Forbidden(_)
    ));
    assert!(
        dispatch(
            "update_org",
            json!({"org":org,"quality_monitoring":{"enabled":true}}),
            &member
        )
        .await
        .is_err()
    );
    assert!(!policy(&db, 1).await.unwrap().enabled);
}

#[tokio::test]
async fn old_consent_generation_cannot_dispatch_after_disable_and_reenable() {
    let p = paid();
    let (db, session) = fixture(p.clone()).await;
    compacted(&db, session, true).await;
    let score = db.claim_trace_scores(32, 120, 3).await.unwrap().remove(0);
    configure(&db, 1, &Default::default()).await.unwrap();
    configure(&db, 1, &p).await.unwrap();
    assert!(
        !db.reserve_monitoring_budget(
            1,
            score.id,
            score.attempts,
            &serde_json::to_value(p).unwrap(),
            1
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn production_judge_admission_dispatches_once_and_refuses_zero_allowance() {
    use crate::domains::observers::judge::LlmJudgeClient;
    use crate::services::ProviderResolverService;
    use crate::storage::encryption::{EncryptionService, generate_encryption_key};
    use crate::storage::{CreateModelRow, CreateProviderRow};
    use everruns_contracts::driver_registry::DriverRegistry;
    use everruns_contracts::provider::DriverId;
    let db = Arc::new(StorageBackend::test_database());
    let encryption =
        Arc::new(EncryptionService::new(&generate_encryption_key("test"), &[]).unwrap());
    let provider = db
        .create_provider(
            1,
            CreateProviderRow {
                name: "admission fixture".into(),
                provider_type: "openai".into(),
                base_url: None,
                api_key_encrypted: Some(encryption.encrypt_string("local-test-only").unwrap()),
                settings: None,
            },
        )
        .await
        .unwrap();
    let model = db
        .create_model(
            1,
            CreateModelRow {
                provider_id: provider.id,
                model_id: "gpt-6.2-sol".into(),
                display_name: "fixture".into(),
                capabilities: vec!["chat".into()],
                enabled: true,
                is_favorite: false,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = captured.clone();
    let mut drivers = DriverRegistry::new();
    drivers.register(DriverId::OpenAI, move |_| {
        let mut config = everruns_llmsim::LlmSimConfig::fixed("fixture response");
        config.message_capture = Some(capture.clone());
        Box::new(everruns_llmsim::LlmSimDriver::new(config))
    });
    let resolver = Arc::new(
        ProviderResolverService::new(db.clone(), Some(encryption))
            .with_driver_registry(drivers.clone()),
    );
    let judge = LlmJudgeClient::new(db.clone(), Arc::new(drivers), resolver);
    let mut p = paid();
    p.model_id = Some(model.id);
    p.daily_budget_usd = 1.0;
    validate_policy(&db, 1, &p).await.unwrap();
    configure(&db, 1, &p).await.unwrap();
    let session = db
        .create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    for zero in [false, true] {
        if zero {
            p.daily_budget_usd = 0.0;
            configure(&db, 1, &p).await.unwrap();
        }
        compacted(&db, session, true).await;
        let score = db
            .claim_trace_scores(32, 120, 3)
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.scorer_key == EXECUTION_KEY)
            .unwrap();
        let call = || {
            judge.judge_compaction(
                1,
                model.id,
                "Assess untrusted data".into(),
                "fixture".into(),
                MonitoringAdmission {
                    score_id: score.id,
                    attempt: score.attempts,
                    policy: &p,
                },
            )
        };
        if zero {
            assert!(
                call()
                    .await
                    .unwrap_err()
                    .is::<crate::domains::observers::judge::MonitoringNotAdmitted>()
            );
        } else {
            assert_eq!(call().await.unwrap().unwrap().text, "fixture response");
            assert!(db.monitoring_spend(1).await.unwrap().reserved_usd > 0.0);
            assert!(
                call()
                    .await
                    .unwrap_err()
                    .is::<crate::domains::observers::judge::MonitoringNotAdmitted>()
            );
        }
    }
    assert_eq!(
        captured.lock().unwrap().len(),
        1,
        "duplicate attempts and exhausted allowance dispatch no model request"
    );
}

#[tokio::test]
async fn dense_tool_activity_cannot_push_the_final_answer_out_of_the_continuation_sample() {
    let (db, session) = fixture(paid()).await;
    let checkpoint = compacted(&db, session, true).await;
    for _ in 0..64 {
        event(
            &db,
            session,
            "tool.completed",
            json!({"result":"routine observation"}),
        )
        .await;
    }
    finish_turn(&db, session).await;
    let row = scores(&db)
        .await
        .into_iter()
        .find(|row| row.scorer_key == CONTINUITY_KEY)
        .unwrap();
    let continued = row.evidence_ref.unwrap()["continuation_sequence"]
        .as_i64()
        .unwrap() as i32;
    let evidence = evidence::gather(&db, session, 1, checkpoint.sequence.unwrap(), continued)
        .await
        .unwrap();
    assert!(evidence.excerpts.iter().any(
        |event| event.event_type == "output.message.completed" && event.text.contains("Friday")
    ));
    assert!(
        evidence
            .excerpts
            .iter()
            .map(|event| event.text.len())
            .sum::<usize>()
            <= evidence::MAX_EVIDENCE_BYTES
    );
}

#[tokio::test]
async fn a_different_turn_cannot_activate_an_old_compaction_assessment() {
    let (db, session) = fixture(paid()).await;
    compacted(&db, session, true).await;
    let other = event(
        &db,
        session,
        "turn.completed",
        json!({"turn_id":everruns_contracts::typed_id::TurnId::new(),"iterations":1}),
    )
    .await;
    assert_eq!(enqueue_event(&db, &other).await.unwrap(), 0);
    let rows = db.claim_trace_scores(32, 120, 3).await.unwrap();
    assert!(rows.iter().all(|row| row.scorer_key == EXECUTION_KEY));
    finish_turn(&db, session).await;
    let rows = db.claim_trace_scores(32, 120, 3).await.unwrap();
    assert!(rows.iter().any(|row| row.scorer_key == CONTINUITY_KEY));
}

#[tokio::test]
async fn consent_and_managed_observer_participate_in_the_command_transaction() {
    let (db, _) = fixture(Default::default()).await;
    let result: anyhow::Result<()> = crate::storage::transaction::scope(
        &db,
        async {
            configure(
                &db,
                1,
                &QualityMonitoringPolicy {
                    enabled: true,
                    ..Default::default()
                },
            )
            .await?;
            assert!(
                policy(&db, 1).await?.enabled,
                "the command sees its staged consent"
            );
            assert_eq!(
                db.system_compaction_observer(1).await?.unwrap().status,
                "active"
            );
            anyhow::bail!("roll back the surrounding command")
        },
        |error| error,
    )
    .await;
    assert!(result.is_err());
    assert!(!policy(&db, 1).await.unwrap().enabled);
    assert_eq!(
        db.system_compaction_observer(1)
            .await
            .unwrap()
            .unwrap()
            .status,
        "paused"
    );
}
