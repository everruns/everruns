//! A data plane's host calls (`host_calls.rs`), on a real database: the
//! org's internal command runs as on a v1 worker, and its answer comes back
//! sealed for the org under the call's own secret.

use everruns_contracts::host_calls::{self, HostCallAnswer, host_call_secret};

use super::*;
use crate::domains::data_plane::host_calls as dispatcher;

/// The org's internal caller, as the route builds it.
fn internal_ctx(cp: &ControlPlane, org_id: i64) -> crate::domains::common::Ctx {
    crate::domains::common::Ctx::new(
        everruns_core::Caller::internal(org_id),
        cp.db.clone(),
        Arc::new(crate::services::CapabilityService::new(cp.db.clone(), None)),
        cp.encryption.clone(),
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
}

async fn ask(cp: &ControlPlane, org_id: i64, command: &str, params: Value) -> HostCallAnswer {
    ask_as(cp, &internal_ctx(cp, org_id), command, params).await
}

/// [`ask`] as `ctx`.
async fn ask_as(
    cp: &ControlPlane,
    ctx: &crate::domains::common::Ctx,
    command: &str,
    params: Value,
) -> HostCallAnswer {
    let row = cp.db.dp_org(ORG).await.unwrap().unwrap();
    let call = host_calls::by_command(command).expect("a declared host call");
    let id = uuid::Uuid::now_v7().to_string();
    let answer = dispatcher::answer(ctx, call, params).await.unwrap();
    let sealed = dispatcher::seal_answer(cp, &row, &host_call_secret(&id), &answer).unwrap();
    assert_eq!(sealed.revision, 0);
    let key = cp.data_key(&row).unwrap();
    let plain = seal::unseal(&key, &org_public(ORG), &sealed).unwrap();
    // Sealed for this call only: under another call's id it does not open.
    let mut other = sealed.clone();
    other.secret_id = host_call_secret("another");
    assert!(seal::unseal(&key, &org_public(ORG), &other).is_err());
    serde_json::from_slice(&plain).unwrap()
}

#[tokio::test]
async fn a_lookup_runs_the_workers_command_and_comes_back_sealed() {
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
        "worker_get_web_search_settings",
        json!({ "session_id": session.to_string() }),
    )
    .await;
    let HostCallAnswer::Value(settings) = answer else {
        panic!("settings, not {answer:?}");
    };
    assert!(settings.is_object(), "{settings}");
}

#[tokio::test]
async fn another_orgs_session_is_not_found_and_only_host_calls_are_allowed() {
    let (cp, _, _) = control_plane().await;
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
        "worker_get_web_search_settings",
        json!({ "session_id": stranger.to_string() }),
    )
    .await;
    assert!(
        matches!(answer, HostCallAnswer::Error { kind: 3, .. }),
        "not found: {answer:?}"
    );

    for command in [
        "worker_get_connection_token",
        "worker_knowledge",
        "worker_poppy_call",
        "worker_get_org_egress_allowlist",
        "slack_action",
        "platform_command_surface",
        "session_sql_db",
    ] {
        assert!(host_calls::by_command(command).is_some(), "{command}");
    }
    for command in [
        "worker_get_sandbox_connection_token",
        "worker_get_mcp_connection_token",
        "worker_get_connection_token_for_connection",
        "session_storage_get",
        "agents_delete",
    ] {
        assert!(
            host_calls::by_command(command).is_none(),
            "{command} is not a host call"
        );
    }
}

/// THREAT[TM-AGENT-017]: a platform call runs as the person the forwarding
/// cell names, only while they are a member, and only for a session that
/// resolves `platform`; without a person it runs as no one.
#[tokio::test]
async fn a_platform_call_runs_as_the_named_member_and_never_as_the_org() {
    const PLATFORM_COMMAND_SURFACE: &str = "platform_command_surface";

    let (cp, _, _) = control_plane().await;
    // The default org, where the built-in harness the v1 worker's test uses
    // lives; the answer is still sealed for ORG's data plane.
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
    let HostCallAnswer::Value(Value::String(output)) = answer else {
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
            matches!(answer, HostCallAnswer::Error { kind: 2, .. }),
            "{why}: {answer:?}"
        );
    }
}

/// A Slack action runs as the session's own endpoint, resolved within the
/// org: a session that did not come through Slack, or another org's, has
/// none, and the refusal comes back as the action's own error.
#[tokio::test]
async fn a_slack_action_needs_the_sessions_own_slack_endpoint() {
    use everruns_contracts::host_calls::SLACK_ACTION;

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
        let HostCallAnswer::Value(answer) = answer else {
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
    let HostCallAnswer::Value(answer) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        answer["error"]["error"],
        json!("invalid_argument"),
        "{answer}"
    );
}

#[tokio::test]
async fn a_sessions_sql_databases_are_the_control_planes_and_the_orgs_only() {
    use everruns_contracts::host_calls::SESSION_SQL_DB;

    let (cp, _, _) = control_plane().await;
    let backend = Arc::new(crate::session_sqldb::InMemorySqlDbBackend::new());
    let store: Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> =
        Arc::new(crate::session_sqldb::InMemorySqlDbStore::new(backend));
    let ctx = internal_ctx(&cp, ORG).with_sqldb_store(store.clone());
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
    let op = |session: everruns_contracts::typed_id::SessionId, op: &str, extra: Value| {
        let mut params = json!({ "session_id": session.to_string(), "op": op });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        params
    };
    let value = |answer: HostCallAnswer| match answer {
        HostCallAnswer::Value(value) => value,
        other => panic!("an SQL answer is a value: {other:?}"),
    };

    let created = value(
        ask_as(
            &cp,
            &ctx,
            SESSION_SQL_DB,
            op(mine, "create", json!({ "name": "notes" })),
        )
        .await,
    );
    assert_eq!(created["ok"]["name"], json!("notes"), "{created}");
    let done = value(
        ask_as(
            &cp,
            &ctx,
            SESSION_SQL_DB,
            op(
                mine,
                "execute",
                json!({ "name": "notes", "sql": "CREATE TABLE t (n INTEGER)" }),
            ),
        )
        .await,
    );
    assert!(done.get("ok").is_some(), "{done}");
    // The console reads the same store the cell's tools wrote.
    let listed = store.list_databases(mine).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "notes");

    // A missing database keeps its kind for the tool.
    let missing = value(
        ask_as(
            &cp,
            &ctx,
            SESSION_SQL_DB,
            op(mine, "query", json!({ "name": "nope", "sql": "SELECT 1" })),
        )
        .await,
    );
    assert_eq!(
        missing["error"]["error"],
        json!("database_not_found"),
        "{missing}"
    );

    // Another org's session is never reached, its store untouched.
    let refused = value(
        ask_as(
            &cp,
            &ctx,
            SESSION_SQL_DB,
            op(stranger, "create", json!({ "name": "x" })),
        )
        .await,
    );
    assert_eq!(refused["error"]["error"], json!("internal"), "{refused}");
    assert!(store.list_databases(stranger).await.unwrap().is_empty());
}
