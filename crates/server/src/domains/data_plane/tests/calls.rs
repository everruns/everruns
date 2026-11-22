//! The injected servers' calls (`calls.rs`), on a real database: the
//! org's internal command runs as on a v1 worker.

use super::*;
use crate::domains::data_plane::calls::{self as dispatcher, Answer, CALLS};

/// The org's internal caller, as the injected servers build it.
fn internal_ctx(cp: &ControlPlane, org_id: i64) -> crate::domains::common::Ctx {
    crate::domains::common::Ctx::new(
        everruns_core::Caller::internal(org_id),
        cp.db.clone(),
        Arc::new(crate::services::CapabilityService::new(cp.db.clone(), None)),
        cp.encryption.clone(),
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
}

/// The call v1 worker command `command` answers.
fn by_command(command: &str) -> Option<&'static dispatcher::Call> {
    CALLS.iter().find(|call| call.command == command)
}

async fn ask(cp: &ControlPlane, org_id: i64, command: &str, params: Value) -> Answer {
    ask_as(cp, &internal_ctx(cp, org_id), command, params).await
}

/// [`ask`] as `ctx`.
async fn ask_as(
    cp: &ControlPlane,
    ctx: &crate::domains::common::Ctx,
    command: &str,
    params: Value,
) -> Answer {
    let call = by_command(command).expect("a call");
    dispatcher::answer(cp, ctx, call, params).await.unwrap()
}

#[tokio::test]
async fn a_call_runs_the_workers_command_for_the_orgs_own_session() {
    let (cp, _, _) = control_plane().await;
    let session = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(ORG))
        .await
        .unwrap()
        .id;
    let answer = ask(
        &cp,
        ORG,
        "worker_get_session_spend",
        json!({ "session_id": session.to_string() }),
    )
    .await;
    let Answer::Value(spend) = answer else {
        panic!("spend, not {answer:?}");
    };
    assert!(spend.is_object(), "{spend}");
    // A session of the default org, asked as ORG.
    let stranger = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(
            everruns_core::DEFAULT_ORG_ID,
        ))
        .await
        .unwrap()
        .id;
    let answer = ask(
        &cp,
        ORG,
        "worker_get_session_spend",
        json!({ "session_id": stranger.to_string() }),
    )
    .await;
    assert!(
        matches!(answer, Answer::Error { kind: 3, .. }),
        "not found: {answer:?}"
    );
}

#[test]
fn only_what_the_injected_servers_serve_is_a_call() {
    use everruns_contracts::runtime::capabilities::injected::CREDENTIAL_CALLS;
    for name in CREDENTIAL_CALLS {
        assert!(dispatcher::by_name(name).is_some(), "{name}");
    }
    // Pushed with the org's config, or kept in the session's shard.
    for command in [
        "worker_get_web_search_settings",
        "worker_get_tool_approval_policy",
        "worker_get_org_egress_allowlist",
        "worker_get_sandbox_connection_token",
        "worker_register_session_resource",
        "worker_upsert_leased_resource",
        "worker_dreaming_state",
        "session_sql_db",
        "agents_delete",
    ] {
        assert!(by_command(command).is_none(), "{command} is not a call");
    }
}

/// THREAT[TM-AGENT-017]: a platform call runs as the token's person, only while they are a member, and only for a session that
/// resolves `platform`; without a person it runs as no one.
#[tokio::test]
async fn a_platform_call_runs_as_the_named_member_and_never_as_the_org() {
    const PLATFORM_COMMAND_SURFACE: &str = "platform_command_surface";

    let (cp, _, _) = control_plane().await;
    // The default org, where the built-in harness the v1 worker's test uses
    // lives.
    let org = everruns_core::DEFAULT_ORG_ID;
    // A session without a harness reads as the org's base harness.
    crate::setup::org_init::initialize_org_harnesses(&cp.db, org)
        .await
        .unwrap();
    let member = cp.db.create_test_user(uuid::Uuid::now_v7()).await;
    cp.db
        .ensure_membership(member, org, "member")
        .await
        .unwrap();
    let outsider = cp.db.create_test_user(uuid::Uuid::now_v7()).await;
    let platform = cp
        .db
        .create_session(crate::storage::CreateSessionRow {
            capabilities: json!([{ "ref": "platform" }]),
            ..crate::storage::test_database::test_session_row(org)
        })
        .await
        .unwrap()
        .id;
    let plain = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(org))
        .await
        .unwrap()
        .id;
    let call = |session: &dyn std::fmt::Display, acting_user: Value| {
        json!({
            "session_id": session.to_string(),
            "input_message_id": uuid::Uuid::now_v7().to_string(),
            "operation": "discover",
            "arguments": { "query": "models" },
            "acting_user": acting_user,
        })
    };

    let answer = ask(
        &cp,
        org,
        PLATFORM_COMMAND_SURFACE,
        call(&platform, json!(member.to_string())),
    )
    .await;
    let Answer::Value(Value::String(output)) = answer else {
        panic!("a member's call runs: {answer:?}");
    };
    assert!(output.contains("list_models"), "{output}");

    // KIND_FORBIDDEN in the internal protocol.
    for (session, acting_user, why) in [
        (platform, Value::Null, "no person: runs as no one"),
        (platform, json!("someone"), "not a user id"),
        (
            platform,
            json!(outsider.to_string()),
            "not a member of the org",
        ),
        (
            plain,
            json!(member.to_string()),
            "the session lacks platform",
        ),
    ] {
        let answer = ask(
            &cp,
            org,
            PLATFORM_COMMAND_SURFACE,
            call(&session, acting_user),
        )
        .await;
        assert!(
            matches!(answer, Answer::Error { kind: 2, .. }),
            "{why}: {answer:?}"
        );
    }
}

/// A Slack action runs as the session's own endpoint, resolved within the
/// org: a session that did not come through Slack, or another org's, has
/// none, and the refusal comes back as the action's own error.
#[tokio::test]
async fn a_slack_action_needs_the_sessions_own_slack_endpoint() {
    use crate::domains::data_plane::calls::SLACK_ACTION;

    let (cp, _, _) = control_plane().await;
    let mine = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(ORG))
        .await
        .unwrap()
        .id;
    let stranger = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(
            everruns_core::DEFAULT_ORG_ID,
        ))
        .await
        .unwrap()
        .id;
    let react = json!({
        "action": "add_reaction", "channel": "C1", "timestamp": "1.2", "name": "eyes"
    });
    for session in [mine, stranger] {
        let answer = ask(
            &cp,
            ORG,
            SLACK_ACTION,
            json!({ "session_id": session.to_string(), "action": react }),
        )
        .await;
        let Answer::Value(answer) = answer else {
            panic!("a refusal is the action's error: {answer:?}");
        };
        assert_eq!(
            answer["error"]["error"],
            json!("no_slack_session"),
            "{answer}"
        );
    }

    let answer = ask(
        &cp,
        ORG,
        SLACK_ACTION,
        json!({ "session_id": mine.to_string(), "action": { "action": "open_dm" } }),
    )
    .await;
    let Answer::Value(answer) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        answer["error"]["error"],
        json!("invalid_argument"),
        "{answer}"
    );
}

/// A v2 session's messages reach its shard without the control plane, so a
/// call acting as the turn's person records the turn's invocation first,
/// as v1 does when the message comes through its API: the person the shard
/// names with their default runtime identity, and the session's agent.
/// The resolver's checks then run on it unchanged.
#[tokio::test]
async fn a_persons_lookup_records_the_turn_it_acts_for() {
    let (cp, _, _) = control_plane().await;
    let member = cp.db.create_test_user(uuid::Uuid::now_v7()).await;
    cp.db
        .ensure_membership(member, ORG, "member")
        .await
        .unwrap();
    let outsider = cp.db.create_test_user(uuid::Uuid::now_v7()).await;
    let session = |on_data_plane: bool| {
        let cp = &cp;
        async move {
            let id = cp
                .db
                .create_session(crate::storage::test_database::test_session_row(ORG))
                .await
                .unwrap()
                .id;
            if on_data_plane {
                let now = chrono::Utc::now();
                cp.db
                    .dp_upsert_session_summary(crate::storage::DpSessionSummary {
                        session_id: &id.to_string(),
                        org_id: ORG,
                        data_plane_id: "dp-1",
                        agent_id: "ak_x",
                        session_version: 0,
                        title: "",
                        status: "started",
                        turns: 0,
                        cost_micros: 0,
                        created_at: now,
                        last_activity_at: now,
                    })
                    .await
                    .unwrap();
            }
            id
        }
    };
    let v2 = session(true).await;
    let v1 = session(false).await;
    let lookup = |session: everruns_contracts::typed_id::SessionId,
                  message: uuid::Uuid,
                  acting_user: Value| {
        json!({
            "session_id": session.to_string(),
            "provider": "linear",
            "input_message_id": message.to_string(),
            "acting_user": acting_user,
        })
    };

    // A member's turn: recorded as them, under their runtime identity.
    let turn = uuid::Uuid::now_v7();
    ask(
        &cp,
        ORG,
        "worker_get_connection_token",
        lookup(v2, turn, json!(member.to_string())),
    )
    .await;
    assert_eq!(
        cp.db
            .runtime_invocation_management_user(v2, turn)
            .await
            .unwrap(),
        Some(member)
    );
    let subject = cp.db.default_virtual_user(ORG, member).await.unwrap().id;
    assert_eq!(
        cp.db.runtime_invocation_subject(v2, turn).await.unwrap(),
        Some(subject)
    );
    // Once per turn: a later call of the same turn keeps it.
    ask(
        &cp,
        ORG,
        "worker_get_connection_token",
        lookup(v2, turn, Value::Null),
    )
    .await;
    assert_eq!(
        cp.db
            .runtime_invocation_management_user(v2, turn)
            .await
            .unwrap(),
        Some(member)
    );

    // No person (a channel message, a trigger): the turn acts as no one.
    let anonymous = uuid::Uuid::now_v7();
    ask(
        &cp,
        ORG,
        "worker_get_service_api_key_connection",
        lookup(v2, anonymous, Value::Null),
    )
    .await;
    assert!(
        cp.db
            .runtime_invocation_exists(v2, anonymous)
            .await
            .unwrap()
    );
    assert!(
        !cp.db
            .runtime_invocation_has_subject(v2, anonymous)
            .await
            .unwrap()
    );

    // Not recorded: an outsider (asks again next time), a v1 session's
    // turn (its own API recorded it), a call that acts as the org.
    for (session, message, command, acting_user, why) in [
        (
            v2,
            uuid::Uuid::now_v7(),
            "worker_get_connection_token",
            json!(outsider.to_string()),
            "not a member",
        ),
        (
            v1,
            uuid::Uuid::now_v7(),
            "worker_get_connection_token",
            json!(member.to_string()),
            "a v1 session",
        ),
        (
            v2,
            uuid::Uuid::now_v7(),
            "worker_knowledge",
            json!(member.to_string()),
            "acts as the org",
        ),
    ] {
        ask(&cp, ORG, command, lookup(session, message, acting_user)).await;
        assert!(
            !cp.db
                .runtime_invocation_exists(session, message)
                .await
                .unwrap(),
            "{why}"
        );
    }
}
