// Session MCP servers reach the in-process worker exactly as they reach the
// gRPC one: in the turn context and in MCP prefix resolution.

use crate::storage::test_database::test_session_row;
use crate::worker_link::direct_worker_adapters::tests::{
    seed_harness_for_platform_store, test_adapters,
};
use everruns_core::session_services::SessionStorageStore;
use everruns_core::{SessionMcpServer, SessionMcpServerSource};
use everruns_internal_protocol::proto::{
    GetMcpServerByPrefixRequest, Uuid as ProtoUuid,
    worker_service_server::WorkerService as GrpcWorkerService,
};
use everruns_worker::worker_adapters::WorkerAdapters;
use std::sync::Arc;

#[tokio::test]
async fn an_ard_attached_server_reaches_in_process_and_grpc_turns() {
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let adapters = test_adapters();
    // The store the gRPC service builds over the same database.
    let storage: Arc<dyn SessionStorageStore> = Arc::new(
        crate::storage::create_db_session_storage_store_without_encryption(
            crate::storage::Database::new(adapters.db.pool().clone()),
        ),
    );
    let adapters = adapters.with_storage_store(storage.clone());
    let harness_id =
        seed_harness_for_platform_store(&adapters.db, org_id, "session-mcp-harness", false).await;
    let mut row = test_session_row(org_id);
    row.harness_id = Some(harness_id);
    let session = adapters.db.create_session(row).await.unwrap();

    everruns_core::put_session_mcp_server(
        storage.as_ref(),
        session.id,
        &SessionMcpServer {
            name: "docs".into(),
            server: crate::kernel_imports::ScopedMcpServer {
                url: "https://docs.example.com/mcp".into(),
                ..Default::default()
            },
            source: SessionMcpServerSource::Ard {
                urn: "urn:ai:example.com:mcp:docs".into(),
            },
        },
    )
    .await
    .unwrap();

    // In process: the turn context carries it, and its tools resolve.
    let turn = adapters
        .load_turn_context(org_id, session.id.uuid())
        .await
        .unwrap();
    assert_eq!(
        turn.session.mcp_servers["docs"].url,
        "https://docs.example.com/mcp"
    );
    let direct = adapters
        .get_mcp_server_by_prefix(org_id, Some(session.id.uuid()), "docs")
        .await
        .expect("in-process prefix resolution");
    assert_eq!(direct.url, "https://docs.example.com/mcp");

    // Over gRPC: prefix resolution agrees.
    let grpc_service = crate::worker_link::grpc_service::WorkerServiceImpl::new(
        adapters.event_service.as_ref().clone(),
        adapters.db.clone(),
        None,
        None,
        crate::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev),
    );
    let grpc = grpc_service
        .get_mcp_server_by_prefix(tonic::Request::new(GetMcpServerByPrefixRequest {
            input_message_id: None,
            org_id,
            session_id: Some(ProtoUuid {
                value: session.id.uuid().to_string(),
            }),
            server_prefix: "docs".to_string(),
        }))
        .await
        .expect("gRPC prefix resolution")
        .into_inner()
        .server
        .expect("gRPC MCP descriptor");
    assert_eq!(grpc.url, direct.url);

    // Removing the record drops it from the next turn on both paths.
    assert!(
        everruns_core::remove_session_mcp_server(storage.as_ref(), session.id, "docs")
            .await
            .unwrap()
    );
    let turn = adapters
        .load_turn_context(org_id, session.id.uuid())
        .await
        .unwrap();
    assert!(turn.session.mcp_servers.is_empty());
    assert!(
        adapters
            .get_mcp_server_by_prefix(org_id, Some(session.id.uuid()), "docs")
            .await
            .is_err()
    );
}
