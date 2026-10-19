//! Reusable native qualification scenarios for third-party session authorities.
//!
//! Enable `hosting-conformance` in a host's **dev-dependencies**. Each scenario
//! uses fresh identities and panics on contract violations. Run against an
//! isolated test authority with no competing application registry. These tests
//! cover the storage/Engine contract; they do not certify crash persistence or
//! independent wakeup delivery. Qualify process loss and platform alarms in the
//! integration's own deployment harness as well.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::spi::{
    EventReadLimit, EventReadRequest, RuntimeMessage,
    actor::{ActorFence, ActorRelease, is_actor_lease_lost},
    events::{EventContext, EventRequest, InputMessageData},
};
mod manual;

use super::{AgentBuild, SessionBinding, SessionHosting};
use crate::{Agent, Engine, Model, SessionId};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// Check immutable identity, admission retries, bounded snapshots, reader
/// isolation and exclusive epochs, including stale mutation refusal after release.
/// The authority must allow a thirty-second lease for this scenario.
pub async fn authority(profile: &impl SessionHosting) {
    let id = SessionId::new();
    let binding = SessionBinding {
        framework: 1,
        behavior: AgentBuild::new("hosting-conformance", uuid::Uuid::new_v4().to_string()),
        harness: None,
        environment: None,
        metadata: json!(null),
    };
    profile.create(id, &binding).await.unwrap();
    profile.create(id, &binding).await.unwrap();
    let mut conflict = binding.clone();
    conflict.behavior.build_id.push_str("-conflict");
    assert!(
        profile.create(id, &conflict).await.is_err(),
        "binding replacement accepted"
    );
    assert_eq!(profile.status(id).await.unwrap().binding, binding);
    let input = RuntimeMessage::user("first");
    let first = profile.admit(id, input.clone(), false).await.unwrap();
    let retry = profile.admit(id, input.clone(), false).await.unwrap();
    assert_eq!(
        first.sequence, retry.sequence,
        "retry appended another input"
    );
    assert_eq!(
        profile
            .receipt(id, input.id)
            .await
            .unwrap()
            .unwrap()
            .sequence,
        first.sequence
    );
    let changed = RuntimeMessage::user("changed").with_id(input.id);
    assert!(
        profile.admit(id, changed, false).await.is_err(),
        "conflicting input retry accepted"
    );
    profile
        .admit(id, RuntimeMessage::user("second"), false)
        .await
        .unwrap();
    let reader = profile.reader(id);
    let snapshot = profile.query_snapshot(id).await.unwrap();
    assert_eq!(snapshot.binding, binding);
    let page = reader
        .event_log
        .read_page(EventReadRequest::new(id, EventReadLimit::new(1).unwrap()))
        .await
        .unwrap();
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.snapshot_high_watermark(), snapshot.high_watermark);
    let first_event = &page.events[0];
    assert_eq!(
        profile
            .event_sequence(id, &first_event.id.to_string())
            .await
            .unwrap(),
        first_event.sequence
    );
    profile
        .admit(id, RuntimeMessage::user("later"), false)
        .await
        .unwrap();
    let next = reader
        .event_log
        .read_page(EventReadRequest::from_cursor(
            page.next_cursor.unwrap(),
            EventReadLimit::new(1).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(next.snapshot_high_watermark(), snapshot.high_watermark);
    assert!(next.next_cursor.is_none(), "snapshot included later input");
    assert!(
        reader
            .event_log
            .read_page(EventReadRequest::new(
                SessionId::new(),
                EventReadLimit::new(1).unwrap()
            ))
            .await
            .is_err(),
        "reader crossed session scope"
    );
    let event = EventRequest::new(
        id,
        EventContext::empty(),
        InputMessageData::new(RuntimeMessage::user("unfenced")),
    );
    assert!(
        reader.event_log.append(event.clone()).await.is_err(),
        "frontend appended canonical history"
    );
    assert!(
        reader
            .storage_store
            .set_value(id, "forbidden", "value")
            .await
            .is_err(),
        "frontend wrote session state"
    );
    let lease = profile
        .claim(id, "conformance-first", Duration::from_secs(30))
        .await
        .unwrap()
        .expect("runnable input was not claimable");
    assert_eq!(lease.binding, binding);
    assert!(
        profile
            .claim(id, "conformance-other", Duration::from_secs(30))
            .await
            .unwrap()
            .is_none(),
        "two owners acquired the session"
    );
    lease
        .epoch
        .observe_fence(&ActorFence::new(id, "conformance-first", lease.actor.fence));
    assert!(
        lease
            .epoch
            .commit("conformance:receipt", json!(1), true)
            .await
            .unwrap()
    );
    assert!(
        !lease
            .epoch
            .commit("conformance:receipt", json!(2), true)
            .await
            .unwrap()
    );
    assert_eq!(
        profile.record(id, "conformance:receipt").await.unwrap(),
        Some(json!(1))
    );
    let ports = lease.epoch.backends();
    ports
        .storage_store
        .set_value(id, "conformance", "retained")
        .await
        .unwrap();
    assert!(matches!(
        lease
            .epoch
            .actor_store()
            .release_actor(
                id,
                "conformance-first",
                lease.actor.fence,
                lease.actor.consumed_seq
            )
            .await
            .unwrap(),
        ActorRelease::Released {
            still_runnable: true
        }
    ));
    let replacement = profile
        .claim(id, "conformance-replacement", Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert!(
        replacement.actor.fence > lease.actor.fence,
        "epoch did not advance"
    );
    let error = lease
        .epoch
        .commit("conformance:stale", json!(true), true)
        .await
        .unwrap_err();
    assert!(
        is_actor_lease_lost(&error),
        "ownership loss did not use the typed signal: {error}"
    );
    assert!(
        ports.event_log.append(event).await.is_err(),
        "stale epoch appended history"
    );
    assert!(
        ports
            .storage_store
            .set_value(id, "conformance", "stale")
            .await
            .is_err(),
        "stale epoch wrote session state"
    );
    let wake_at = profile.status(id).await.unwrap().now_ms + 60_000;
    assert!(
        lease.epoch.schedule(Some(wake_at)).await.is_err(),
        "stale epoch replaced wakeup"
    );
    assert!(
        lease.epoch.suspend("stale").await.is_err(),
        "stale epoch suspended replacement"
    );
    assert_eq!(profile.record(id, "conformance:stale").await.unwrap(), None);
    assert_eq!(
        profile
            .reader(id)
            .storage_store
            .get_value(id, "conformance")
            .await
            .unwrap()
            .as_deref(),
        Some("retained")
    );
    assert!(
        replacement
            .epoch
            .commit("conformance:replacement", json!(true), true)
            .await
            .unwrap()
    );
    replacement
        .epoch
        .actor_store()
        .release_actor(
            id,
            "conformance-replacement",
            replacement.actor.fence,
            replacement.actor.consumed_seq,
        )
        .await
        .unwrap();
}

/// Check authority-clock lease expiry without releasing the owner. Refuse old
/// receipts and storage writes before another owner claims, then recover the
/// original input under a higher epoch. Requires one-second leases.
pub async fn lease_expiry(profile: &impl SessionHosting) {
    let id = SessionId::new();
    let binding = SessionBinding {
        framework: 1,
        behavior: AgentBuild::new("hosting-expiry", uuid::Uuid::new_v4().to_string()),
        harness: None,
        environment: None,
        metadata: json!(null),
    };
    profile.create(id, &binding).await.unwrap();
    profile
        .admit(id, RuntimeMessage::user("survive owner loss"), false)
        .await
        .unwrap();
    let first = profile
        .claim(id, "expired-owner", Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    let ports = first.epoch.backends();
    first
        .epoch
        .commit("conformance:before-expiry", json!(true), true)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while profile.status(id).await.unwrap().executing {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("authority lease did not expire");
    let error = first
        .epoch
        .commit("conformance:after-expiry", json!(true), true)
        .await
        .unwrap_err();
    assert!(
        is_actor_lease_lost(&error),
        "expired epoch did not report ownership loss"
    );
    assert!(
        ports
            .storage_store
            .set_value(id, "conformance", "expired")
            .await
            .is_err()
    );
    let replacement = profile
        .claim(id, "new-owner", Duration::from_secs(30))
        .await
        .unwrap()
        .expect("expired work was not claimable");
    assert!(replacement.actor.fence > first.actor.fence);
    assert_eq!(
        profile
            .record(id, "conformance:after-expiry")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        profile
            .record(id, "conformance:before-expiry")
            .await
            .unwrap(),
        Some(json!(true))
    );
    replacement
        .epoch
        .actor_store()
        .release_actor(
            id,
            "new-owner",
            replacement.actor.fence,
            replacement.actor.consumed_seq,
        )
        .await
        .unwrap();
}

/// Drive an ordinary Framework Agent through mixed approval/client waits,
/// destroy its Engine, reopen it, reject an invalid batch without side effects,
/// and finalize exactly once. Uses a deterministic provider, with a real tool
/// effect counted in Rust. This is handle/reconstruction qualification, not an
/// OS process-loss proof.
pub async fn engine_wait_recovery(profile: impl SessionHosting + Clone) {
    // Own activation explicitly so process schedulers cannot retain the Engine
    // while it is parked on a long-lived approval deadline.
    let profile = manual::Manual(profile);
    let build = AgentBuild::new("hosting-conformance", uuid::Uuid::new_v4().to_string());
    let effects = Arc::new(AtomicUsize::new(0));
    let counted = effects.clone();
    let agent = Agent::builder()
        .instructions("Use the tools, then finish.")
        .model(Model::simulated_with_config(
            crate::LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
                vec![
                    crate::ToolCall {
                        id: "mutate_first".into(),
                        name: "mutate".into(),
                        arguments: serde_json::json!({}),
                    },
                    crate::ToolCall {
                        id: "render_first".into(),
                        name: "render".into(),
                        arguments: serde_json::json!({}),
                    },
                ],
                vec![crate::ToolCall {
                    id: "mutate_retry".into(),
                    name: "mutate".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ]),
        ))
        .client_tool(
            "render",
            "Render the result",
            serde_json::json!({"type":"object"}),
        )
        .tool(
            crate::FunctionTool::new(
                "mutate",
                "Change one value",
                serde_json::json!({"type":"object"}),
                move |_: serde_json::Value| {
                    let counted = counted.clone();
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, String>(serde_json::json!({"done":true}))
                    }
                },
            )
            .always_needs_approval(),
        )
        .durable_approvals()
        .build()
        .unwrap();
    let engine = |agent: Agent| {
        Engine::builder()
            .hosting(profile.clone())
            .register(build.clone(), agent)
            .build()
    };
    let owner = engine(agent.clone());
    let id = SessionId::new();
    let session = owner.create_registered(id, &build).await.unwrap();
    let message = crate::MessageId::new();
    session.send_with_id(message, "work").await.unwrap();
    owner.activate(id).await.unwrap();
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let status = profile.status(id).await.unwrap();
            if status.waiting.len() == 2 {
                break status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(session);
    drop(owner);
    let replacement = Engine::builder()
        .hosting(profile.clone())
        .register(build.clone(), agent)
        .register(
            AgentBuild::new(&build.agent, "newer-deployment-build"),
            Agent::builder()
                .instructions("Use the newer behavior.")
                .model(Model::simulated("wrong build"))
                .build()
                .unwrap(),
        )
        .build();
    let session = replacement.resume(id).await.unwrap();
    assert!(
        matches!(session.admission(message).await.unwrap().unwrap().state().await.unwrap(),
        crate::hosting::AdmissionState::Waiting { calls, .. } if calls.len() == 2)
    );
    assert!(session.interrupted_turn().await.unwrap().is_none());
    assert!(session.resume_interrupted_turn().await.unwrap().is_none());
    let turn = waiting.open_turn.unwrap();
    let approval = waiting
        .waiting
        .iter()
        .find(|call| call.name == "approve_tool_call")
        .unwrap();
    let decisions = vec![crate::hosting::ApprovalCommit {
        turn_id: turn,
        request_id: approval.id.clone(),
        decision: crate::hosting::ToolApprovalDecision::Allow,
    }];
    let before = session.events_after(0).await.unwrap().len();
    let result = crate::ToolAnswer {
        tool_call_id: "render_first".into(),
        tool_name: "render".into(),
        result: serde_json::json!({"rendered":true}),
        error: None,
    };
    assert!(
        session
            .resolve_waits(
                uuid::Uuid::new_v4(),
                crate::TurnId::new(),
                decisions.clone(),
                vec![result.clone()]
            )
            .await
            .is_err()
    );
    let mut invalid = result.clone();
    invalid.tool_call_id = "not_pending".into();
    assert!(
        session
            .resolve_waits(uuid::Uuid::new_v4(), turn, decisions.clone(), vec![invalid])
            .await
            .is_err()
    );
    assert_eq!(session.events_after(0).await.unwrap().len(), before);
    assert_eq!(session.pending_tools().await.unwrap().len(), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    let command = uuid::Uuid::new_v4();
    session
        .resolve_waits(command, turn, decisions.clone(), vec![result.clone()])
        .await
        .unwrap();
    replacement.activate(id).await.unwrap();
    let sent = session.admission(message).await.unwrap().unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(10), sent.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    let canonical = session.events_after(0).await.unwrap();
    let first = &canonical[0];
    assert_eq!(
        session.event_sequence(&first.event_id).await.unwrap(),
        first.sequence()
    );
    let after = canonical.len();
    session
        .resolve_waits(command, turn, decisions, vec![result])
        .await
        .unwrap();
    assert_eq!(session.events_after(0).await.unwrap().len(), after);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

/// Qualify deployment with a missing build, restoration beside a newer build,
/// and retry of the original admission without changing its immutable binding.
pub async fn retained_build_recovery(profile: impl SessionHosting + Clone) {
    use super::SessionCompatibility;
    let profile = manual::Manual(profile);
    let old = AgentBuild::new("retained-deployment", uuid::Uuid::new_v4().to_string());
    let new = AgentBuild::new(&old.agent, uuid::Uuid::new_v4().to_string());
    let behavior = |reply: &str| {
        Agent::builder()
            .instructions("Answer briefly.")
            .model(Model::simulated(reply))
            .build()
            .unwrap()
    };
    let id = SessionId::new();
    let message = crate::MessageId::new();
    {
        let before = Engine::builder()
            .hosting(profile.clone())
            .register(old.clone(), behavior("original behavior"))
            .build();
        before
            .create_registered(id, &old)
            .await
            .unwrap()
            .send_with_id(message, "accepted before deploy")
            .await
            .unwrap();
    }
    let missing = Engine::builder()
        .hosting(profile.clone())
        .register(new.clone(), behavior("new behavior"))
        .build();
    assert_eq!(
        missing.session_compatibility(id).await.unwrap(),
        SessionCompatibility::MissingBuild {
            required: old.clone()
        }
    );
    assert!(missing.activate(id).await.is_err());
    let suspended = missing.session_status(id).await.unwrap();
    assert!(suspended.suspended && !suspended.executing);
    assert_eq!(
        suspended.suspension_reason.as_deref(),
        Some("retained_build_unavailable")
    );
    assert!(
        missing
            .retry_session(id, uuid::Uuid::new_v4())
            .await
            .is_err()
    );
    let readable = missing.resume(id).await.unwrap();
    assert_eq!(readable.events_after(0).await.unwrap().len(), 1);
    drop(readable);
    drop(missing);

    // A single compatible Engine serves both retained behaviors. No older
    // Engine binary, copied transcript or overwritten build ID is required.
    let restored = Engine::builder()
        .hosting(profile.clone())
        .register(old.clone(), behavior("original behavior"))
        .register(new.clone(), behavior("new behavior"))
        .build();
    assert!(
        restored
            .session_compatibility(id)
            .await
            .unwrap()
            .is_available()
    );
    let command = uuid::Uuid::new_v4();
    restored.retry_session(id, command).await.unwrap();
    assert_eq!(
        restored.session_binding(id).await.unwrap(),
        suspended.binding
    );
    restored.activate(id).await.unwrap();
    let session = restored.resume(id).await.unwrap();
    let admission = session.admission(message).await.unwrap().unwrap();
    assert_eq!(
        admission.wait().await.unwrap().response,
        "original behavior"
    );
    let events = session.events_after(0).await.unwrap();
    restored.retry_session(id, command).await.unwrap();
    restored.activate(id).await.unwrap();
    assert_eq!(session.events_after(0).await.unwrap().len(), events.len());
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type() == "input.message")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type() == "turn.started")
            .count(),
        1
    );
    let fresh = restored
        .create_registered(SessionId::new(), &new)
        .await
        .unwrap();
    let admission = fresh.send("new session").await.unwrap();
    restored.activate(fresh.session_id()).await.unwrap();
    assert_eq!(admission.wait().await.unwrap().response, "new behavior");
}
