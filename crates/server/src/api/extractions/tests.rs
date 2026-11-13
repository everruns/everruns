use super::*;
#[test]
fn rejects_provider_and_model_override() {
    for selector in [
        serde_json::json!({"provider":"other"}),
        serde_json::json!({"model":"unconfigured"}),
    ] {
        let mut input =
            serde_json::json!({"state":"Invoice 42", "fields":{"number":{"type":"text"}}});
        input
            .as_object_mut()
            .unwrap()
            .extend(selector.as_object().unwrap().clone());
        let request: ExtractRequest =
            serde_json::from_value(serde_json::json!({"model_id":"unused", "input":input}))
                .unwrap();
        assert!(validate_selection(&request).is_err());
    }
}
fn state() -> ApiState {
    let db = std::sync::Arc::new(crate::storage::StorageBackend::test_database());
    let auth = crate::auth::AuthState::builtin(crate::auth::AuthConfig::default(), db.clone());
    ApiState::for_test(db, None, auth)
}

fn request() -> ExtractRequest {
    serde_json::from_value(serde_json::json!({
        "model_id": ModelId::new().to_string(),
        "input": {"state": "Invoice", "fields": {"total": {"type": "number"}}}
    }))
    .unwrap()
}

#[tokio::test]
async fn member_and_read_only_owner_cannot_spend_provider_credentials() {
    for (role, read_only) in [
        (everruns_core::OrgRole::Member, false),
        (everruns_core::OrgRole::Owner, true),
    ] {
        let org = ResolvedOrg {
            org_id: 1,
            public_id: "org_test".into(),
            name: "Test".into(),
            user_id: None,
            role,
            is_platform_user: false,
            feature_flags: crate::domains::common::all_feature_flags_for_test(),
            read_only,
        };
        let error = extract_typed_fields(org, State(state()), Json(request()))
            .await
            .unwrap_err();
        assert_eq!(error.0, axum::http::StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn another_organizations_model_is_not_visible_to_extraction() {
    let state = state();
    let provider = state
        .db
        .create_provider(
            2,
            crate::storage::CreateProviderRow {
                name: "Other org provider".into(),
                provider_type: "openrouter".into(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    let model = state
        .db
        .create_model(
            2,
            crate::storage::CreateModelRow {
                provider_id: provider.id,
                model_id: "cloudflare/clairic".into(),
                display_name: "Clairic".into(),
                capabilities: vec!["extractions".into()],
                is_favorite: false,
                enabled: true,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    let org = ResolvedOrg {
        org_id: 1,
        public_id: "org_test".into(),
        name: "Test".into(),
        user_id: None,
        role: everruns_core::OrgRole::Owner,
        is_platform_user: false,
        feature_flags: crate::domains::common::all_feature_flags_for_test(),
        read_only: false,
    };
    let mut input = request();
    input.model_id = model.id.to_string();
    let error = extract_typed_fields(org, State(state), Json(input))
        .await
        .unwrap_err();
    assert_eq!(error.0, axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn route_requires_authentication() {
    use tower::ServiceExt;
    let db = std::sync::Arc::new(crate::storage::StorageBackend::test_database());
    let auth = crate::auth::AuthState::builtin(
        crate::auth::AuthConfig {
            mode: crate::auth::config::AuthMode::Full,
            ..Default::default()
        },
        db.clone(),
    );
    let response = routes(ApiState::for_test(db, None, auth))
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/extractions")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&request()).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn configured_org_key_and_model_reach_native_provider_without_chat() {
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/alpha/extractions"))
        .and(header("authorization", "Bearer synthetic-org-key"))
        .and(body_json(serde_json::json!({
            "model": "cloudflare/clairic", "state": "Invoice INV-42", "fields": {"invoice": {"type": "text"}}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "cloudflare/clairic", "answers": {"invoice": {"type": "text", "value": "INV-42", "span": [8, 14]}}
        })))
        .expect(1).mount(&upstream).await;
    let db = Arc::new(crate::storage::StorageBackend::test_database());
    let encryption = Arc::new(
        crate::storage::encryption::EncryptionService::new(
            "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            &[],
        )
        .unwrap(),
    );
    let provider = db
        .create_provider(
            1,
            crate::storage::CreateProviderRow {
                name: "Synthetic extraction provider".into(),
                provider_type: "openrouter".into(),
                base_url: Some(format!("{}/api/v1", upstream.uri())),
                api_key_encrypted: Some(encryption.encrypt_string("synthetic-org-key").unwrap()),
                settings: None,
            },
        )
        .await
        .unwrap();
    let model = db
        .create_model(
            1,
            crate::storage::CreateModelRow {
                provider_id: provider.id,
                model_id: "cloudflare/clairic".into(),
                display_name: "Clairic".into(),
                capabilities: vec!["extractions".into()],
                is_favorite: false,
                enabled: true,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    let auth = crate::auth::AuthState::builtin(crate::auth::AuthConfig::default(), db.clone());
    let state = ApiState::for_test(db, Some(encryption), auth);
    let org = ResolvedOrg {
        org_id: 1,
        public_id: "org_test".into(),
        name: "Test".into(),
        user_id: None,
        role: everruns_core::OrgRole::Owner,
        is_platform_user: false,
        feature_flags: crate::domains::common::all_feature_flags_for_test(),
        read_only: false,
    };
    let result = extract_typed_fields(
        org,
        State(state),
        Json(ExtractRequest {
            model_id: model.id.to_string(),
            input: ExtractionRequest::new("Invoice INV-42").field(
                "invoice",
                everruns_contracts::extractions::ExtractionField::text(),
            ),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(result.answers["invoice"].value, "INV-42");
    assert_eq!(result.answers["invoice"].span, Some([8, 14]));
    assert_eq!(result.answers["invoice"].confidence, None);
}

#[tokio::test]
async fn disabled_inactive_managed_and_missing_key_accounts_fail_before_dispatch() {
    for (enabled, status, managed, expected) in [
        (false, "active", false, "Select an enabled extraction model"),
        (true, "disabled", false, "Provider is not active"),
        (
            true,
            "active",
            true,
            "Extractions require an organization-owned provider account",
        ),
        (true, "active", false, "Provider credentials are required"),
    ] {
        let state = state();
        let provider = state
            .db
            .create_provider(
                1,
                crate::storage::CreateProviderRow {
                    name: "Guard fixture".into(),
                    provider_type: "openrouter".into(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        state
            .db
            .update_provider(
                1,
                provider.id.uuid(),
                crate::storage::UpdateProvider {
                    status: Some(status.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        state
            .db
            .set_provider_managed(1, provider.id.uuid(), managed)
            .await
            .unwrap();
        let model = state
            .db
            .create_model(
                1,
                crate::storage::CreateModelRow {
                    provider_id: provider.id,
                    model_id: "cloudflare/clairic".into(),
                    display_name: "Clairic".into(),
                    capabilities: vec!["extractions".into()],
                    is_favorite: false,
                    enabled,
                    source: "manual".into(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap();
        let org = ResolvedOrg {
            org_id: 1,
            public_id: "org_test".into(),
            name: "Test".into(),
            user_id: None,
            role: everruns_core::OrgRole::Owner,
            is_platform_user: false,
            feature_flags: crate::domains::common::all_feature_flags_for_test(),
            read_only: false,
        };
        let mut input = request();
        input.model_id = model.id.to_string();
        let (status, Json(error)) = extract_typed_fields(org, State(state), Json(input))
            .await
            .unwrap_err();
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert!(serde_json::to_string(&error).unwrap().contains(expected));
    }
}
