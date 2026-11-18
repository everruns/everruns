//! A tool's call-time lookups on the data plane (data plane v2, step
//! 11.6b): an executor asks through its directory cell, the cell passes the
//! request to the control plane over the link, and the answer comes back
//! sealed with the org's data key for the executor to open.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::Arc;

use everruns_cell::dp::LookupAnswer;
use everruns_cell::providers::OrgMaterial;
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::SessionId;
use everruns_core::connection_services::UserConnectionResolver;
use serde_json::json;
use support::org::{DATA_KEY, DATA_KEY_ID, ORG, org_executor, org_fleet, session};

/// The sandbox keys a cell resolves itself: none here.
struct NoKeys;

#[async_trait::async_trait]
impl UserConnectionResolver for NoKeys {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(None)
    }

    async fn get_sandbox_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
        _credential: &SessionSandboxCredential,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(Some("sandbox-key".into()))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tools_lookup_reaches_the_control_plane_and_comes_back_for_its_org() {
    let (fleet, cp) = org_fleet(&[]).await;
    cp.answer_lookups(ORG, DATA_KEY, DATA_KEY_ID);
    cp.set_lookup(
        "worker_get_connection_token",
        LookupAnswer::Value(json!("gh-token-1")),
    );

    // A turn on the executor, which then knows the session's org.
    let session = session(&fleet, &cp).await;
    fleet.say(&session, "ping").await;
    fleet.turns(&session, 1).await;
    let id: SessionId = session.parse().unwrap();

    let material: Arc<dyn OrgMaterial> = org_executor(&fleet);
    let resolver =
        everruns_cell::lookups::connections(Arc::new(NoKeys), Some(&Arc::downgrade(&material)), id);
    let token = resolver.get_connection_token(id, "github").await.unwrap();
    assert_eq!(token.as_deref(), Some("gh-token-1"));
    let asked = cp.lookups();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].0, ORG);
    assert_eq!(asked[0].1, "worker_get_connection_token");
    assert_eq!(asked[0].2["session_id"], json!(session));
    assert_eq!(asked[0].2["provider"], json!("github"));

    // The control plane's refusal is the tool's failure, never a default.
    let error = resolver.get_web_search_settings(id).await.unwrap_err();
    assert!(error.to_string().contains("Session not found"), "{error}");

    // Sandbox keys and MCP grants stay with the data plane.
    assert_eq!(
        resolver
            .get_sandbox_connection_token(id, "daytona", &SessionSandboxCredential::default())
            .await
            .unwrap()
            .as_deref(),
        Some("sandbox-key")
    );
    assert_eq!(
        resolver
            .get_connection_token(id, "mcp_oauth_docs")
            .await
            .unwrap(),
        None
    );
    assert_eq!(cp.lookups().len(), 2);

    // The org's knowledge, for the `knowledge` tools, the same way.
    cp.set_lookup(
        "worker_knowledge",
        LookupAnswer::Value(json!({ "results": [], "warnings": ["empty"], "truncated": false })),
    );
    let backends = everruns_cell::lookups::attach(
        everruns_core::host::HostBackends::in_memory(),
        Arc::new(NoKeys),
        Some(&Arc::downgrade(&material)),
        id,
    );
    let factory = backends
        .tool_context_extensions_factory
        .expect("knowledge installed");
    let knowledge = factory(0, id)
        .get::<everruns_contracts::knowledge::KnowledgeRuntimeExt>()
        .expect("the knowledge runtime");
    let query: everruns_contracts::knowledge::KnowledgeQuery =
        serde_json::from_value(json!({ "query": "refunds" })).unwrap();
    let found = knowledge.0.search(id, query).await.unwrap();
    assert_eq!(found.warnings, vec!["empty".to_owned()]);
    let asked = cp.lookups();
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert_eq!(asked[2].1, "worker_knowledge");
    assert_eq!(asked[2].2["operation"], json!("search"));
    assert_eq!(asked[2].2["query"]["query"], json!("refunds"));

    // A session the executor does not know is no org's: nothing is asked.
    let stranger = SessionId::new();
    let resolver = everruns_cell::lookups::connections(
        Arc::new(NoKeys),
        Some(&Arc::downgrade(&material)),
        stranger,
    );
    assert_eq!(
        resolver
            .get_connection_token(stranger, "github")
            .await
            .unwrap(),
        None
    );
    assert_eq!(cp.lookups().len(), 3);
}
