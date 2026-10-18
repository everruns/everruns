//! Service MCP provenance follows the invocation's responder (EVE-1252).

use super::*;
use everruns_contracts::typed_id::MessageId;

/// An agent with its own identity (and principal) and `mcp_servers`.
async fn agent_with_identity(
    db: &StorageBackend,
    mcp_servers: serde_json::Value,
) -> (Uuid, VirtualUserId) {
    let identity_id = db.create_test_virtual_user(DEFAULT_ORG_ID).await;
    db.create_principal(CreatePrincipalRow {
        id: PrincipalId::new(),
        org_id: DEFAULT_ORG_ID,
        kind: "virtual_user".to_string(),
        subject_id: Some(identity_id.uuid()),
        parent_principal_id: None,
        resolved_user_id: None,
        metadata: serde_json::json!({}),
    })
    .await
    .unwrap();
    let agent_id = db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await;
    sqlx::query("UPDATE agents SET virtual_user_id = $1, mcp_servers = $2 WHERE id = $3")
        .bind(identity_id.uuid())
        .bind(mcp_servers)
        .bind(agent_id)
        .execute(db.database().pool())
        .await
        .unwrap();
    (agent_id, identity_id)
}

fn acting_subject_of(event: &Event) -> Option<String> {
    event
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("acting_principal_id"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// EVE-1252: a guest agent answers in the host agent's session. The
/// connection resolver signs the service call in as the invocation's
/// responder, so provenance must name the responder, using the
/// responder's attachments, not the session agent's.
#[tokio::test]
async fn service_mcp_event_acts_as_invocation_responder() {
    let db = Arc::new(StorageBackend::test_database());
    let event_service = EventService::new(db.clone(), EventDelivery::in_memory());
    // The host has an identity but no `linear` attachment.
    let (host_id, _) = agent_with_identity(&db, serde_json::json!({})).await;
    let (guest_id, guest_identity) = agent_with_identity(
        &db,
        serde_json::json!({ "linear": { "use": "catalog:linear", "actsAs": "service" } }),
    )
    .await;
    let mut input = service_session_input(None);
    input.agent_id = Some(AgentId::from_uuid(host_id));
    input.mcp_servers = serde_json::json!({});
    let session = db.create_session(input).await.unwrap();
    let message = Uuid::now_v7();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        session.id,
        message,
        None,
        None,
        Some(guest_id),
    )
    .await
    .unwrap();
    let mut request = service_tool_event(session.id);
    request.context.input_message_id = Some(MessageId::from_uuid(message));

    let event = event_service.emit(request).await.unwrap();

    let acting_principal_id: PrincipalId = acting_subject_of(&event)
        .expect("responder's service call names an actor")
        .parse()
        .unwrap();
    let acting = db
        .get_principal(DEFAULT_ORG_ID, acting_principal_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acting.subject_id, Some(guest_identity.uuid()));
}

/// A worker-supplied invocation id is trusted only when it is one of the
/// session's own invocations; otherwise it could pick another session's
/// responder as the actor.
#[tokio::test]
async fn service_mcp_event_rejects_unknown_invocation() {
    let db = Arc::new(StorageBackend::test_database());
    let event_service = EventService::new(db.clone(), EventDelivery::in_memory());
    let (agent_id, _) = agent_with_identity(
        &db,
        serde_json::json!({ "linear": { "use": "catalog:linear", "actsAs": "service" } }),
    )
    .await;
    let mut input = service_session_input(None);
    input.agent_id = Some(AgentId::from_uuid(agent_id));
    let session = db.create_session(input).await.unwrap();
    let other = db
        .create_session(service_session_input(None))
        .await
        .unwrap();
    let foreign = Uuid::now_v7();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        other.id,
        foreign,
        None,
        None,
        Some(agent_id),
    )
    .await
    .unwrap();
    let mut request = service_tool_event(session.id);
    request.context.input_message_id = Some(MessageId::from_uuid(foreign));

    let error = event_service.emit(request).await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains("service MCP event names an unknown invocation")
    );
}

/// A service server attached at run time (ARD or a session record) is a
/// service attachment too, so its events carry the actor stamp.
#[tokio::test]
async fn service_mcp_event_recognizes_run_time_session_records() {
    let db = Arc::new(StorageBackend::test_database());
    let event_service = EventService::new(db.clone(), EventDelivery::in_memory());
    let (agent_id, identity) = agent_with_identity(&db, serde_json::json!({})).await;
    let mut input = service_session_input(None);
    input.agent_id = Some(AgentId::from_uuid(agent_id));
    input.mcp_servers = serde_json::json!({});
    let session = db.create_session(input).await.unwrap();
    let storage =
        crate::storage::DbSessionStorageStore::new_without_encryption(db.database().clone());
    everruns_core::session_mcp_servers::put_session_mcp_server(
        &storage,
        session.id,
        &everruns_core::session_mcp_servers::SessionMcpServer {
            name: "linear".to_string(),
            server: serde_json::from_value(
                serde_json::json!({ "use": "catalog:linear", "actsAs": "service" }),
            )
            .unwrap(),
            source: everruns_core::session_mcp_servers::SessionMcpServerSource::Ard {
                urn: "urn:test:linear".to_string(),
            },
        },
    )
    .await
    .unwrap();

    let event = event_service
        .emit(service_tool_event(session.id))
        .await
        .unwrap();

    let acting_principal_id: PrincipalId = acting_subject_of(&event)
        .expect("run-time service attachment names an actor")
        .parse()
        .unwrap();
    let acting = db
        .get_principal(DEFAULT_ORG_ID, acting_principal_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acting.subject_id, Some(identity.uuid()));
}
