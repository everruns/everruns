//! Real worker preparation and API-command projection share PostgreSQL KV;
//! only the outbound MCP boundary is a gated fixture. No LLM is needed.
use super::{seed_harness_for_platform_store, test_adapters};
use crate::domains::common::{Command, Ctx};
use crate::domains::mcp_servers::scoped_mcp::CatalogPreviewEgress;
use crate::domains::mcp_servers::scoped_mcp::Readiness;
use crate::domains::mcp_servers::session_servers::ListChatMcpServers;
use crate::storage::{DbSessionStorageStore, test_database::test_session_row};
use everruns_core::{
    Caller, DEFAULT_ORG_ID, DefaultPermissionResolver, EgressRequest, EgressResponse, EgressResult,
    EgressService, EgressStreamResponse, ScopedMcpServer, SessionMcpServer, SessionMcpServerSource,
};
use everruns_worker::worker_adapters::WorkerAdapters;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Semaphore;

struct GatedMcp {
    delegate: CatalogPreviewEgress,
    slow_gate: Semaphore,
    slow_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl EgressService for GatedMcp {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let listing = serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["method"]
            == "tools/list";
        if listing && request.url.ends_with("/slow") {
            self.slow_calls.fetch_add(1, Ordering::SeqCst);
            self.slow_gate
                .acquire()
                .await
                .expect("gate remains open")
                .forget();
        }
        self.delegate.send(request).await
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter(vec![Ok(response.body)])),
        })
    }
}

#[tokio::test]
async fn platform_readiness_worker_to_api_smoke() {
    Readiness::with_test_budget(Duration::from_millis(100), async {
        let adapters = test_adapters();
        let harness =
            seed_harness_for_platform_store(&adapters.db, DEFAULT_ORG_ID, "readiness-smoke", false)
                .await;
        let mut row = test_session_row(DEFAULT_ORG_ID);
        row.harness_id = Some(harness);
        let session = adapters.db.create_session(row).await.unwrap().id;
        let storage = Arc::new(DbSessionStorageStore::new_without_encryption(
            adapters.db.database().clone(),
        ));
        for (name, path) in [("a_slow", "slow"), ("z_fast", "fast")] {
            everruns_core::put_session_mcp_server(
                storage.as_ref(),
                session,
                &SessionMcpServer {
                    name: name.into(),
                    server: ScopedMcpServer {
                        url: format!("http://8.8.8.8/{path}"),
                        ..Default::default()
                    },
                    source: SessionMcpServerSource::UserMcp,
                },
            )
            .await
            .unwrap();
        }
        let egress = Arc::new(GatedMcp {
            delegate: CatalogPreviewEgress::default(),
            slow_gate: Semaphore::new(0),
            slow_calls: AtomicUsize::new(0),
        });
        let adapters = adapters
            .with_storage_store(storage)
            .with_egress_service(egress.clone());
        let ctx = Ctx::minimal(
            Caller::internal(DEFAULT_ORG_ID),
            adapters.db.clone(),
            None,
            Arc::new(DefaultPermissionResolver),
        );
        let first = tokio::time::timeout(
            Duration::from_secs(5),
            adapters.load_turn_context(DEFAULT_ORG_ID, session.uuid()),
        )
        .await
        .expect("pending discovery must not block worker preparation")
        .unwrap();
        assert!(
            first
                .mcp_tool_definitions
                .iter()
                .any(|tool| tool.name().starts_with("mcp_z_fast_"))
        );
        assert!(
            first
                .mcp_tool_definitions
                .iter()
                .all(|tool| !tool.name().starts_with("mcp_a_slow_"))
        );
        assert_eq!(egress.slow_calls.load(Ordering::SeqCst), 1);
        // This command is exactly the projection invoked by GET
        // /v1/sessions/{session_id}/mcp-servers, using the same persisted records.
        let pending = ListChatMcpServers {
            session_id: session.to_string(),
        }
        .run(&ctx)
        .await
        .unwrap();
        assert_eq!(
            pending
                .iter()
                .find(|server| server.name == "a_slow")
                .unwrap()
                .discovery
                .as_deref(),
            Some("pending")
        );
        assert_eq!(
            pending
                .iter()
                .find(|server| server.name == "z_fast")
                .unwrap()
                .discovery
                .as_deref(),
            Some("ready")
        );
        egress.slow_gate.add_permits(1);
        let second = tokio::time::timeout(
            Duration::from_secs(5),
            adapters.load_turn_context(DEFAULT_ORG_ID, session.uuid()),
        )
        .await
        .expect("later worker preparation completes")
        .unwrap();
        assert!(
            second
                .mcp_tool_definitions
                .iter()
                .any(|tool| tool.name().starts_with("mcp_a_slow_"))
        );
        assert!(
            second
                .mcp_tool_definitions
                .iter()
                .any(|tool| tool.name().starts_with("mcp_z_fast_"))
        );
        let ready = ListChatMcpServers {
            session_id: session.to_string(),
        }
        .run(&ctx)
        .await
        .unwrap();
        assert_eq!(
            ready
                .iter()
                .find(|server| server.name == "a_slow")
                .unwrap()
                .discovery
                .as_deref(),
            Some("ready")
        );
        assert_eq!(
            egress.slow_calls.load(Ordering::SeqCst),
            1,
            "later turn must harvest the original discovery"
        );
    })
    .await;
}
