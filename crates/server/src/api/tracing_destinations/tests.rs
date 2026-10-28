use crate::domains::common::{Command, dispatch};
use crate::domains::trace_exports::{test_support::*, *};
use crate::storage::StorageBackend;
use everruns_core::{Caller, OrgRole};
use serde_json::json;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn customer_tracing_test_endpoint_uses_configured_host_egress_and_permissions() {
    use crate::{
        api::{state::ApiState, tracing_destinations::test_destination},
        auth::{AuthConfig, AuthState, ResolvedOrg},
        services::CapabilityService,
    };
    use axum::extract::{Path, State};
    let db = Arc::new(StorageBackend::test_database());
    let id = destination(&db, 1).await;
    let collector = Arc::new(Collector::default());
    let mut context = ctx(db.clone(), OrgRole::Owner);
    // ApiState's context may have no optional egress override; the host boundary is on capabilities.
    assert!(context.egress_service.is_none());
    context.capability_service = Arc::new(
        CapabilityService::new(db.clone(), Some(encryption()))
            .with_mcp_egress_service(collector.clone()),
    );
    let mut state = ApiState::new(context, AuthState::builtin(AuthConfig::default(), db));
    let org = |role, read_only| ResolvedOrg {
        org_id: 1,
        public_id: "org_00000000000000000000000000000001".into(),
        name: "Test".into(),
        user_id: None,
        role,
        is_platform_user: false,
        feature_flags: Default::default(),
        read_only,
    };
    for (role, read_only) in [(OrgRole::Member, false), (OrgRole::Owner, true)] {
        let failure = test_destination(org(role, read_only), State(state.clone()), Path(id))
            .await
            .err()
            .unwrap();
        assert_eq!(failure.0, axum::http::StatusCode::FORBIDDEN);
    }
    let result = test_destination(org(OrgRole::Owner, false), State(state.clone()), Path(id))
        .await
        .unwrap();
    assert!(result.0.accepted);
    assert_eq!(collector.requests.lock().unwrap().len(), 1);
    let failure = test_destination(org(OrgRole::Owner, false), State(state.clone()), Path(id))
        .await
        .err()
        .unwrap();
    assert_eq!(failure.0, axum::http::StatusCode::TOO_MANY_REQUESTS);
    // An embedder's explicit context boundary takes precedence over the shared fallback.
    let other_id = destination(&state.db, 1).await;
    let preferred = Arc::new(Collector::default());
    let mut explicit = ctx(state.db.clone(), OrgRole::Owner);
    explicit.capability_service = state.capability_service.clone();
    explicit.egress_service = Some(preferred.clone());
    let overridden = ApiState::new(explicit, state.auth.clone());
    let response = test_destination(
        org(OrgRole::Owner, false),
        State(overridden),
        Path(other_id),
    )
    .await
    .unwrap();
    assert!(response.0.accepted);
    assert_eq!(preferred.requests.lock().unwrap().len(), 1);
    assert_eq!(collector.requests.lock().unwrap().len(), 1);
    struct DenyAll;
    impl everruns_core::PermissionResolver for DenyAll {
        fn has_permission(&self, _: &Caller, _: &everruns_core::Permission) -> bool {
            false
        }
        fn caller_permissions(&self, _: &Caller) -> Vec<everruns_core::Permission> {
            vec![]
        }
    }
    state.auth.permission_resolver = Arc::new(DenyAll);
    let failure = test_destination(org(OrgRole::Owner, false), State(state), Path(id))
        .await
        .err()
        .unwrap();
    assert_eq!(failure.0, axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn customer_tracing_destination_history_records_rotation_but_restores_only_safe_fields() {
    use crate::{
        api::{
            state::ApiState,
            tracing_destinations::{SaveDestinationRequest, update_destination},
        },
        auth::{AuthConfig, AuthState, ResolvedOrg},
    };
    use axum::{
        Json,
        extract::{Path, State},
    };
    let db = Arc::new(StorageBackend::test_database());
    let id = destination(&db, 1).await;
    let owner = ctx(db.clone(), OrgRole::Owner);
    destination_history::UpdateTracingDestinationMetadata {
        id,
        name: Some("Reviewed collector".into()),
        enabled: None,
    }
    .run(&owner)
    .await
    .unwrap();
    let mut config = config();
    config.name = "Reviewed collector".into();
    config.capture_content = true;
    let state = ApiState::new(
        owner.clone(),
        AuthState::builtin(AuthConfig::default(), db.clone()),
    );
    let org = ResolvedOrg {
        org_id: 1,
        public_id: owner.caller.org_public_id.clone(),
        name: "Test".into(),
        user_id: None,
        role: OrgRole::Owner,
        is_platform_user: false,
        feature_flags: Default::default(),
        read_only: false,
    };
    let updated = update_destination(
        org,
        State(state),
        Path(id),
        Json(SaveDestinationRequest {
            config,
            revision: Some(2),
            secrets: Some(TraceSecrets {
                headers: [("Authorization".into(), "Bearer ROTATED_CUSTOMER_KEY".into())].into(),
            }),
        }),
    )
    .await
    .unwrap();
    assert_eq!(updated.0.revision, 3);
    let history = dispatch(
        "list_entity_history",
        json!({"entity_ref":id,"kind":"tracing_destination"}),
        &owner,
    )
    .await
    .unwrap();
    assert!(!history.contains("CUSTOMER_KEY"));
    let entries: serde_json::Value = serde_json::from_str(&history).unwrap();
    assert_eq!(entries[0]["revision"], 3);
    let restored = dispatch(
        "restore_entity_revision",
        json!({"entity_ref":id,"kind":"tracing_destination","revision":1,"expected_revision":3}),
        &owner,
    )
    .await
    .unwrap();
    assert!(
        restored.contains("authentication"),
        "rotation is warned about, never undone"
    );
    assert!(!restored.contains("CUSTOMER_KEY"));
    let row = db.tracing_destination(1, id).await.unwrap().unwrap();
    let current: DestinationConfig = serde_json::from_value(row.config).unwrap();
    assert_eq!(current.name, "Customer collector");
    assert!(
        current.capture_content,
        "protected content policy must remain current"
    );
    let secrets = db
        .tracing_secrets(1, row.connection_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        encryption()
            .decrypt_to_string(&secrets)
            .unwrap()
            .contains("ROTATED_CUSTOMER_KEY")
    );
}
