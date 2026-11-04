use super::behavior::observe_session;
use super::classifiers::{AGENT_STUCK, SUSPECTED_ABUSE};
use super::fixes::*;
use super::inspector::*;
use crate::domains::common::{Command, Ctx};
use crate::domains::evals::EvalService;
use crate::domains::evals::experiments::types::{EvalExperiment, Verdict};
use crate::domains::evals::record::EvalTarget;
use crate::domains::evals::types::{CreateEvalCaseRequest, CreateEvalRequest};
use crate::records::feature_flags::FeatureFlags;
use crate::storage::StorageBackend;
use crate::storage::test_database::test_session_row;
use async_trait::async_trait;
use everruns_contracts::typed_id::{AgentId, SessionId};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const INSTRUCTIONS: &str = "Check the sign-in claim every five minutes and report its state.";
const STOP_RULE: &str =
    "Stop and delete this schedule once the claim is approved, expired, or after 12 runs.";

fn stuck_answer(add: &str) -> String {
    json!({
        "what_happened": "The schedule polls a claim that expired and repeats the same error.",
        "severity": "medium",
        "suggested_action": "Add a stop rule.",
        "cluster": null,
        "cluster_title": "Expired claim polling",
        "instruction_change": {"summary": "Add a stop rule to the schedule's instructions", "add": add}
    })
    .to_string()
}

#[derive(Default)]
struct FakeInspector {
    replies: Mutex<VecDeque<String>>,
    prompts: Mutex<Vec<String>>,
}

#[async_trait]
impl InspectorModel for FakeInspector {
    async fn write_up(
        &self,
        _org_id: i64,
        _system: String,
        prompt: String,
    ) -> anyhow::Result<Option<InspectorReply>> {
        self.prompts.lock().unwrap().push(prompt);
        Ok(self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .map(|text| InspectorReply {
                text,
                model: "muse-test".into(),
                input_tokens: 900,
                output_tokens: 120,
                cost_usd: Some(0.002),
            }))
    }
}

fn answering(replies: Vec<String>) -> FakeInspector {
    FakeInspector {
        replies: Mutex::new(replies.into()),
        ..Default::default()
    }
}

fn ctx(db: &Arc<StorageBackend>) -> Ctx {
    let flags = FeatureFlags {
        evals: true,
        ..Default::default()
    };
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None)
        .with_feature_flags(flags)
}

async fn agent(db: &StorageBackend) -> Uuid {
    let agent = db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await;
    sqlx::query("UPDATE agents SET system_prompt = $2 WHERE id = $1")
        .bind(agent)
        .bind(INSTRUCTIONS)
        .execute(db.database().pool())
        .await
        .unwrap();
    agent
}

/// A finding of `code` about a session of `agent`.
async fn finding(db: &StorageBackend, code: &'static str, agent: Uuid) -> Uuid {
    let session = db
        .create_session(test_session_row(DEFAULT_ORG_ID))
        .await
        .unwrap()
        .id
        .uuid();
    sqlx::query("UPDATE sessions SET agent_id = $2 WHERE id = $1")
        .bind(session)
        .bind(agent)
        .execute(db.database().pool())
        .await
        .unwrap();
    observe_session(
        db,
        code,
        DEFAULT_ORG_ID,
        session,
        None,
        json!({"probability": 0.97, "calibrated": true}),
    )
    .await
    .unwrap();
    let subject = SessionId::from_uuid(session).to_string();
    db.open_subject_health_issues(code)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.subject_id.as_deref() == Some(&subject))
        .expect("finding opened")
        .id
}

async fn fix_of(ctx: &Ctx, issue: Uuid) -> Option<FindingFix> {
    ListFindingFixes {
        issue_id: Some(issue),
        agent_id: None,
        open: false,
    }
    .run(ctx)
    .await
    .unwrap()
    .fixes
    .into_iter()
    .next()
}

async fn mark_verified(db: &StorageBackend, fix: &FindingFix) {
    let row = db
        .get_finding_fix(DEFAULT_ORG_ID, &fix.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        db.move_finding_fix(row.id, &["proposed"], "verified", None, None, None, None)
            .await
            .unwrap()
    );
}

async fn instructions(db: &StorageBackend, agent: Uuid) -> String {
    db.get_agent(DEFAULT_ORG_ID, AgentId::from_uuid(agent))
        .await
        .unwrap()
        .unwrap()
        .system_prompt
}

#[test]
fn a_write_up_carries_a_fix_only_within_its_bounds() {
    let w = parse_write_up(&stuck_answer(STOP_RULE), 0).unwrap();
    let (summary, add) = w.fix.unwrap();
    assert_eq!(summary, "Add a stop rule to the schedule's instructions");
    assert_eq!(add, STOP_RULE);
    assert!(
        parse_write_up(&stuck_answer(&"x".repeat(ADDITION_CHARS + 1)), 0)
            .unwrap()
            .fix
            .is_none()
    );
    assert!(
        parse_write_up(&stuck_answer("  "), 0)
            .unwrap()
            .fix
            .is_none()
    );
    let none = r#"{"what_happened": "x", "severity": "low", "instruction_change": null}"#;
    assert!(parse_write_up(none, 0).unwrap().fix.is_none());
}

#[test]
fn an_addition_goes_after_the_instructions() {
    assert_eq!(
        apply_to("Be brief.\n", " Stop after 12 runs. "),
        "Be brief.\n\nStop after 12 runs."
    );
    assert_eq!(apply_to("", "Stop."), "Stop.");
}

fn experiment(status: &str, broke: u32, verdict: &str) -> EvalExperiment {
    let cell = |label: &str, passed: u32| json!({"label": label, "passed": passed, "trials": 2, "cost_usd": 0.1});
    serde_json::from_value(json!({
        "id": "evalexp_x", "eval_id": "eval_x", "name": "Fix", "dataset_version": 2,
        "trials": 2, "baseline": CURRENT, "status": status, "variants": [],
        "created_at": "2026-11-02T00:00:00Z",
        "comparison": {"baseline": CURRENT,
            "verdicts": [{"label": WITH_FIX, "paired_cases": 2, "fixed": 1, "broke": broke, "verdict": verdict}],
            "cases": [
                {"case_id": "c1", "case_name": "one", "changed": true, "cells": [cell(CURRENT, 0), cell(WITH_FIX, 2)]},
                {"case_id": "c2", "case_name": "two", "changed": false, "cells": [cell(CURRENT, 2), cell(WITH_FIX, 2 - broke)]}
            ]}
    }))
    .unwrap()
}

#[test]
fn a_finished_check_counts_both_sides_and_passes_only_without_regressions() {
    assert_eq!(
        judge(&experiment("running", 0, "within_noise"), "e"),
        CheckOutcome::Running
    );
    assert_eq!(
        judge(&experiment("failed", 0, "within_noise"), "e"),
        CheckOutcome::Failed
    );
    let CheckOutcome::Done(good) = judge(&experiment("completed", 0, "within_noise"), "Claims")
    else {
        panic!("a completed check is judged");
    };
    assert_eq!(good.eval_name, "Claims");
    assert_eq!((good.current.passed, good.with_fix.passed), (1, 2));
    assert_eq!(good.current.pass_rate, 0.5);
    assert_eq!(good.with_fix.pass_rate, 1.0);
    assert_eq!(good.fixed, 1);
    assert_eq!(status_after(&good), "verified");
    let CheckOutcome::Done(bad) = judge(&experiment("completed", 1, "within_noise"), "e") else {
        panic!("a completed check is judged");
    };
    assert_eq!(status_after(&bad), "regressed");
    let mut noisy = good.clone();
    noisy.verdict = Verdict::Regression;
    assert_eq!(status_after(&noisy), "regressed");
}

#[tokio::test]
async fn the_inspector_suggests_a_fix_that_applies_only_after_a_clean_check() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx(&db);
    let agent = agent(&db).await;
    let issue = finding(&db, AGENT_STUCK, agent).await;
    let model = answering(vec![stuck_answer(STOP_RULE)]);
    assert_eq!(sweep(&db, &model).await.unwrap().written, 1);
    // It read the agent's instructions to write the fix.
    let prompt = model.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains(INSTRUCTIONS), "{prompt}");

    let fix = fix_of(&ctx, issue).await.expect("a fix was written");
    assert_eq!(fix.status, "proposed");
    assert_eq!(fix.addition, STOP_RULE);
    assert!(fix.current);
    assert_eq!(fix.agent_id, AgentId::from_uuid(agent).to_string());

    // Not checked: it cannot be applied, and with no eval it cannot be checked.
    let err = ApplyFindingFix {
        fix_id: fix.id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("Check the fix"), "{err}");
    let err = CheckFindingFix {
        fix_id: fix.id.clone(),
        req: CheckFindingFixRequest::default(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("no eval with cases"), "{err}");
    assert_eq!(instructions(&db, agent).await, INSTRUCTIONS);

    // Once checked clean, applying adds the text as a new revision, once.
    mark_verified(&db, &fix).await;
    let applied = ApplyFindingFix {
        fix_id: fix.id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(applied.status, "applied");
    assert_eq!(
        instructions(&db, agent).await,
        format!("{INSTRUCTIONS}\n\n{STOP_RULE}")
    );
    let err = ApplyFindingFix {
        fix_id: fix.id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("already applied"), "{err}");
}

#[tokio::test]
async fn a_fix_checked_against_older_instructions_is_not_applied() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx(&db);
    let agent = agent(&db).await;
    let issue = finding(&db, AGENT_STUCK, agent).await;
    sweep(&db, &answering(vec![stuck_answer(STOP_RULE)]))
        .await
        .unwrap();
    let fix = fix_of(&ctx, issue).await.unwrap();
    mark_verified(&db, &fix).await;
    sqlx::query("UPDATE agents SET system_prompt = 'Someone rewrote this.' WHERE id = $1")
        .bind(agent)
        .execute(db.database().pool())
        .await
        .unwrap();
    assert!(!fix_of(&ctx, issue).await.unwrap().current);
    let err = ApplyFindingFix {
        fix_id: fix.id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("changed after the check"), "{err}");
    assert_eq!(instructions(&db, agent).await, "Someone rewrote this.");
    // An edit sends it back to be checked; a rejection keeps the note.
    let edited = UpdateFindingFix {
        fix_id: fix.id.clone(),
        req: UpdateFindingFixRequest {
            addition: "Stop after 12 runs with the same result.".into(),
            summary: None,
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(edited.status, "proposed");
    assert_eq!(edited.addition, "Stop after 12 runs with the same result.");
    let rejected = RejectFindingFix {
        fix_id: fix.id.clone(),
        req: RejectFindingFixRequest {
            note: "Fixing the schedule itself".into(),
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(rejected.status, "rejected");
    assert_eq!(
        rejected.decision_note.as_deref(),
        Some("Fixing the schedule itself")
    );
}

#[tokio::test]
async fn a_check_runs_the_agents_eval_with_and_without_the_fix() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx(&db);
    let agent = agent(&db).await;
    let service = EvalService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let eval = service
        .create(
            &caller,
            CreateEvalRequest {
                name: "Claims".into(),
                description: None,
                target: Some(EvalTarget::Session {
                    harness_id: None,
                    harness_name: None,
                    agent_id: Some(AgentId::from_uuid(agent)),
                    model_id: None,
                    system_prompt: None,
                    max_iterations: None,
                }),
                model_override: None,
                tags: None,
            },
        )
        .await
        .unwrap();
    let case: CreateEvalCaseRequest = serde_json::from_value(json!({
        "name": "Expired claim",
        "conversation": [{"role": "user", "content": "The claim expired."}],
        "scorers": [{"type": "contains", "text": "stop"}]
    }))
    .unwrap();
    service
        .create_case(&caller, &eval.public_id.to_string(), case)
        .await
        .unwrap();
    let issue = finding(&db, AGENT_STUCK, agent).await;
    sweep(&db, &answering(vec![stuck_answer(STOP_RULE)]))
        .await
        .unwrap();
    let fix = fix_of(&ctx, issue).await.unwrap();

    let checking = CheckFindingFix {
        fix_id: fix.id.clone(),
        req: CheckFindingFixRequest {
            trials: Some(1),
            ..Default::default()
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(checking.status, "checking");
    assert_eq!(
        checking.eval_id.as_deref(),
        Some(eval.public_id.to_string().as_str())
    );
    let experiment = db
        .get_eval_experiment_by_public_id(
            DEFAULT_ORG_ID,
            checking.experiment_id.as_deref().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    let runs = db.list_eval_experiment_runs(experiment.id).await.unwrap();
    let with_fix = runs
        .iter()
        .find(|r| r.variant_label.as_deref() == Some(WITH_FIX))
        .expect("a variant with the fix");
    let target: EvalTarget = serde_json::from_value(with_fix.target.clone().unwrap()).unwrap();
    let EvalTarget::Session {
        system_prompt,
        agent_id,
        ..
    } = target
    else {
        panic!("a session target");
    };
    assert_eq!(system_prompt.as_deref(), Some(STOP_RULE));
    assert_eq!(agent_id, Some(AgentId::from_uuid(agent)));
    // The agent is unchanged until someone applies the fix.
    assert_eq!(instructions(&db, agent).await, INSTRUCTIONS);
}

#[tokio::test]
async fn suspected_abuse_never_gets_a_fix() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx(&db);
    let agent = agent(&db).await;
    let issue = finding(&db, SUSPECTED_ABUSE, agent).await;
    let model = answering(vec![stuck_answer(STOP_RULE)]);
    sweep(&db, &model).await.unwrap();
    assert!(!model.prompts.lock().unwrap()[0].contains(INSTRUCTIONS));
    assert!(fix_of(&ctx, issue).await.is_none());
}
