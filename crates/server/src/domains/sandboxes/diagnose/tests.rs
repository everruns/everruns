use super::*;
use serde_json::json;

use crate::domains::sandbox_templates::record::DEFAULT_DELETE_AFTER_SECONDS;

fn at(minutes: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minutes)
}

/// A pinned profile whose `lifecycle` is `lifecycle` as stored (fields the
/// JSON leaves out take their serde defaults, as for a real snapshot).
fn profile(lifecycle: serde_json::Value) -> serde_json::Value {
    use crate::domains::sandbox_templates::record::*;
    let spec = ResolvedSandboxSpec {
        template_revision_id: None,
        target: SandboxTargetSpec::managed("daytona"),
        containment: SandboxContainmentSpec::isolated(),
        durability: SandboxDurability::Checkpointed,
        lifecycle: SandboxLifecycle::default(),
        bootstrap: SandboxBootstrap::default(),
    };
    let mut value = serde_json::to_value(spec).unwrap();
    value["lifecycle"] = lifecycle;
    value
}

fn inputs(
    observed: &str,
    session: Option<&str>,
    lifecycle: serde_json::Value,
) -> SandboxLifecycleInputs {
    SandboxLifecycleInputs {
        observed_state: observed.to_string(),
        desired_state: if observed == "paused" {
            "paused"
        } else {
            "ready"
        }
        .to_string(),
        lifecycle: Some(lifecycle.clone()),
        profile: Some(profile(lifecycle)),
        session_status: session.map(str::to_string),
        session_updated_at: None,
        last_activity_at: Some(at(0)),
        updated_at: at(0),
    }
}

fn sandbox(state: &str) -> SandboxFleetItem {
    SandboxFleetItem {
        id: "sandbox_01".into(),
        state: state.into(),
        desired_state: if state == "paused" { "paused" } else { "ready" }.into(),
        provider: "daytona".into(),
        target_kind: Some("managed".into()),
        role: "primary".into(),
        session_id: Some("session_01".into()),
        session_title: None,
        agent_id: None,
        agent_name: None,
        template: None,
        generation: 1,
        checkpoint_count: 0,
        external_id: Some("dtn-1".into()),
        workspace_path: None,
        idle_after_seconds: Some(300),
        size: None,
        image: None,
        last_init_error: None,
        attention: vec![],
        created_at: at(-60),
        updated_at: at(0),
        last_activity_at: Some(at(0)),
        deleted_at: None,
    }
}

fn provider(state: &str, active: bool, auto_stop: Option<i64>) -> ProviderSandboxState {
    ProviderSandboxState::new(
        "daytona",
        ProviderSandboxView {
            external_id: "dtn-1".into(),
            exists: true,
            state: state.into(),
            active: Some(active),
            auto_stop_minutes: auto_stop,
            ..ProviderSandboxView::default()
        },
    )
}

fn diagnose(
    now: DateTime<Utc>,
    sandbox: &SandboxFleetItem,
    lifecycle: Option<&SandboxLifecycleStatus>,
    work: &SandboxSessionWork,
    provider: Option<&ProviderSandboxState>,
    events: &[SandboxEvent],
) -> Vec<SandboxFinding> {
    findings(&DiagnosisInputs {
        now,
        sandbox,
        lifecycle,
        work,
        provider,
        events,
        orphan_sighted_at: None,
    })
}

fn ids(found: &[SandboxFinding]) -> Vec<&str> {
    found.iter().map(|f| f.id.as_str()).collect()
}

#[test]
fn running_idle_sandbox_has_timer_and_sweep_deadlines() {
    let status = lifecycle_status(&inputs(
        "ready",
        Some("idle"),
        json!({"idle_after_seconds": 300}),
    ))
    .unwrap();
    assert_eq!(status.idle_action, "checkpoint_and_stop");
    assert_eq!(status.pause_due_at, Some(at(5)));
    assert_eq!(status.sweep_pause_at, Some(at(6)));
    assert_eq!(status.stale_turn_pause_at, None);
    assert_eq!(status.delete_due_at, None);
}

#[test]
fn active_session_defers_to_the_stale_turn_pause() {
    let status = lifecycle_status(&inputs(
        "ready",
        Some("active"),
        json!({"idle_after_seconds": 300}),
    ))
    .unwrap();
    assert_eq!(status.pause_due_at, None);
    assert_eq!(status.stale_turn_pause_at, Some(at(6 * 60)));
}

#[test]
fn keep_running_sets_no_pause() {
    let status = lifecycle_status(&inputs(
        "ready",
        Some("idle"),
        json!({"idle_after_seconds": 300, "idle_action": "keep_running"}),
    ))
    .unwrap();
    assert_eq!(status.pause_due_at, None);
    assert_eq!(status.stale_turn_pause_at, None);
}

#[test]
fn paused_sandbox_gets_its_retention_deadline() {
    let status = lifecycle_status(&inputs(
        "paused",
        Some("idle"),
        json!({"idle_after_seconds": 300, "delete_after_seconds": 3600}),
    ))
    .unwrap();
    assert_eq!(status.delete_due_at, Some(at(60)));
    let default = lifecycle_status(&inputs("paused", Some("idle"), json!({}))).unwrap();
    assert_eq!(
        default.delete_due_at,
        Some(at(0) + Duration::seconds(DEFAULT_DELETE_AFTER_SECONDS as i64))
    );
    let kept = lifecycle_status(&inputs(
        "paused",
        Some("idle"),
        json!({"delete_after_seconds": null}),
    ))
    .unwrap();
    assert_eq!(kept.delete_due_at, None);
    let delete = lifecycle_status(&inputs(
        "paused",
        Some("idle"),
        json!({"idle_action": "delete"}),
    ))
    .unwrap();
    assert_eq!(delete.delete_due_at, Some(at(0)));
}

#[test]
fn overdue_running_sandbox_is_a_warning() {
    let status = lifecycle_status(&inputs(
        "ready",
        Some("idle"),
        json!({"idle_after_seconds": 300}),
    ))
    .unwrap();
    let found = diagnose(
        at(30),
        &sandbox("running"),
        Some(&status),
        &SandboxSessionWork::default(),
        None,
        &[],
    );
    assert!(ids(&found).contains(&"lifecycle.overdue"));
    assert_eq!(worst(&found), CheckStatus::Warning);
    let fresh = diagnose(
        at(3),
        &sandbox("running"),
        Some(&status),
        &SandboxSessionWork::default(),
        None,
        &[],
    );
    assert!(!ids(&fresh).contains(&"lifecycle.overdue"));
    assert_eq!(worst(&fresh), CheckStatus::Ok);
}

#[test]
fn provider_stop_before_everruns_idle_pause_is_explained() {
    let status = lifecycle_status(&inputs(
        "ready",
        Some("idle"),
        json!({"idle_after_seconds": 1800}),
    ))
    .unwrap();
    let daytona = provider("stopped", false, Some(15));
    let found = diagnose(
        at(3),
        &sandbox("running"),
        Some(&status),
        &SandboxSessionWork::default(),
        Some(&daytona),
        &[],
    );
    let ids = ids(&found);
    assert!(ids.contains(&"provider.stopped_early"));
    assert!(ids.contains(&"provider.auto_stop"));
    let stopped = found
        .iter()
        .find(|f| f.id == "provider.stopped_early")
        .unwrap();
    assert!(stopped.why.as_deref().unwrap().contains("15 minutes"));
}

#[test]
fn paused_record_with_a_running_provider_sandbox_is_a_problem() {
    let found = diagnose(
        at(3),
        &sandbox("paused"),
        None,
        &SandboxSessionWork::default(),
        Some(&provider("started", true, Some(15))),
        &[],
    );
    assert!(ids(&found).contains(&"provider.still_running"));
    assert_eq!(found[0].status, CheckStatus::Problem, "problems sort first");
}

#[test]
fn a_failure_counts_until_the_same_action_succeeds() {
    let event = |minute, outcome: &str| SandboxEvent {
        at: at(minute),
        generation: 1,
        actor: "idle_sweep".into(),
        action: "pause".into(),
        outcome: outcome.into(),
        reason: "provider_error".into(),
        message: "Idle pause failed".into(),
        detail: json!({}),
    };
    let work = SandboxSessionWork::default();
    let failed = diagnose(
        at(10),
        &sandbox("running"),
        None,
        &work,
        None,
        &[event(1, "failed")],
    );
    assert!(ids(&failed).contains(&"events.failure"));
    let recovered = diagnose(
        at(10),
        &sandbox("paused"),
        None,
        &work,
        None,
        &[event(1, "failed"), event(2, "done")],
    );
    assert!(!ids(&recovered).contains(&"events.failure"));
}

#[test]
fn a_long_unclaimed_turn_is_flagged() {
    let work = SandboxSessionWork {
        session_status: Some("active".into()),
        phase: Some("running".into()),
        phase_since: Some(at(0)),
        claimed: Some(false),
        ..Default::default()
    };
    let found = diagnose(at(120), &sandbox("running"), None, &work, None, &[]);
    assert!(ids(&found).contains(&"session.stuck_turn"));
    let claimed = SandboxSessionWork {
        claimed: Some(true),
        ..work
    };
    let found = diagnose(at(120), &sandbox("running"), None, &claimed, None, &[]);
    assert!(!ids(&found).contains(&"session.stuck_turn"));
}

#[test]
fn data_plane_sandboxes_say_who_manages_them() {
    let work = SandboxSessionWork {
        data_plane: true,
        ..Default::default()
    };
    let found = diagnose(at(3), &sandbox("not_started"), None, &work, None, &[]);
    assert!(ids(&found).contains(&"data_plane.lifecycle"));
}

#[test]
fn provider_free_text_is_scrubbed() {
    let state = ProviderSandboxState::new(
        "daytona",
        ProviderSandboxView {
            external_id: "dtn-1".into(),
            exists: true,
            state: "error".into(),
            error: Some("proxy https://s1.org.sess.1.abc@egress.example.com refused; api_key=sk-abcdefghijklmnopqrstuvwx".into()),
            ..ProviderSandboxView::default()
        },
    );
    let error = state.error.unwrap();
    assert!(!error.contains("s1.org.sess.1.abc"), "{error}");
    assert!(!error.contains("sk-abcdefghijklmnopqrstuvwx"), "{error}");
}

#[test]
fn shard_reads_map_to_work_and_sandbox() {
    let (work, sandbox) = shard_work(&json!({
        "work": {"turn_running": true, "inbox": 2, "claimed": false, "attempts": 1, "phase": "running"},
        "sandbox": {"provider": "daytona", "external_id": "dtn-9", "status": "running"}
    }));
    assert!(work.data_plane);
    assert_eq!(work.session_status.as_deref(), Some("active"));
    assert_eq!(work.pending_inputs, Some(2));
    assert_eq!(sandbox.unwrap()["external_id"], "dtn-9");
    let (_, none) = shard_work(&json!({"work": {}, "sandbox": null}));
    assert!(none.is_none());
}
#[test]
fn provider_lookup_uses_the_shards_current_primary_resource() {
    let shard = serde_json::json!({"external_id": "current-sandbox"});
    assert_eq!(
        super::provider_external_id(Some("old-sandbox"), true, Some(&shard)),
        Some("current-sandbox")
    );
    assert_eq!(
        super::provider_external_id(Some("old-sandbox"), true, None),
        None
    );
    assert_eq!(
        super::provider_external_id(Some("resource-sandbox"), false, Some(&shard)),
        Some("resource-sandbox")
    );
}
