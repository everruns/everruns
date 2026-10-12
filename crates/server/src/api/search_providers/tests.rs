use super::*;
use crate::auth::AuthState;
use crate::auth::config::AuthConfig;
use crate::storage::{CreateOrganizationConnectionRow, EncryptionService, StorageBackend};
use everruns_core::DEFAULT_ORG_ID;
use serde_json::json;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

fn org(role: OrgRole) -> ResolvedOrg {
    ResolvedOrg {
        org_id: DEFAULT_ORG_ID,
        public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        name: "Test".to_string(),
        user_id: Some(Uuid::now_v7()),
        role,
        is_platform_user: false,
        feature_flags: crate::domains::common::all_feature_flags_for_test(),
    }
}

/// An org with one Brave search account (with a key), one without a key, and
/// a sandbox provider account that web search must ignore.
async fn fixture() -> (AppState, Uuid, Uuid) {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = Arc::new(EncryptionService::new(TEST_KEY, &[]).unwrap());
    let account = |provider: &str, name: &str, key: Vec<u8>| CreateOrganizationConnectionRow {
        org_id: DEFAULT_ORG_ID,
        name: name.to_string(),
        provider: provider.to_string(),
        access_token_encrypted: key,
        provider_username: None,
        provider_metadata: None,
    };
    let team = db
        .create_organization_connection(account(
            "brave_search",
            "Team",
            encryption.encrypt_string("brave-key").unwrap(),
        ))
        .await
        .unwrap()
        .id;
    let other = db
        .create_organization_connection(account(
            "brave_search",
            "Zeta",
            encryption.encrypt_string("other-key").unwrap(),
        ))
        .await
        .unwrap()
        .id;
    db.create_organization_connection(account("daytona", "Sandboxes", vec![1]))
        .await
        .unwrap();
    let auth = AuthState::builtin(AuthConfig::default(), db.clone());
    let state = AppState::new(
        db,
        Some(encryption),
        auth,
        crate::platform::oss_connector_registry(),
    );
    (state, team, other)
}

async fn preview(
    state: &AppState,
    role: OrgRole,
    body: serde_json::Value,
) -> Result<SearchPlanPreview, ApiError> {
    preview_search_plan(
        State(state.clone()),
        org(role),
        Json(serde_json::from_value(body).unwrap()),
    )
    .await
    .map(|Json(preview)| preview)
}

fn step_ids(preview: &SearchPlanPreview) -> Vec<String> {
    preview.steps.iter().map(|step| step.id.clone()).collect()
}

#[tokio::test]
async fn catalog_lists_built_ins_search_connectors_and_accounts_only() {
    let (state, team, other) = fixture().await;
    let Json(catalog) = list_search_providers(State(state.clone()), org(OrgRole::Member))
        .await
        .unwrap();
    let ids: Vec<&str> = catalog.built_in.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["free", "native"]);
    assert!(catalog.built_in[1].price_note.is_some());
    assert!(
        catalog
            .connectors
            .iter()
            .any(|connector| connector.provider == "brave_search")
    );
    assert!(
        catalog
            .connectors
            .iter()
            .all(|connector| connector.provider != "daytona"),
        "sandbox connectors are not search providers"
    );
    let accounts: Vec<Uuid> = catalog.accounts.iter().map(|a| a.id).collect();
    assert_eq!(accounts, [team, other]);
    assert_eq!(catalog.accounts[0].status, "ready");
    // Automatic: native first, then the accounts in listing order.
    assert_eq!(catalog.accounts[0].automatic_position, Some(2));
    assert_eq!(catalog.accounts[1].automatic_position, Some(3));
    assert_eq!(catalog.defaults, WebSearchDefaults::default());
}

#[tokio::test]
async fn preview_follows_the_current_org_defaults() {
    let (state, team, other) = fixture().await;
    let plan = preview(&state, OrgRole::Member, json!({})).await.unwrap();
    assert_eq!(plan.provider, SearchProviderSetting::Auto);
    // No default model: no native step.
    assert!(!plan.model_has_native);
    assert_eq!(
        step_ids(&plan),
        [
            format!("account:{team}"),
            format!("account:{other}"),
            "free".into()
        ]
    );
    assert_eq!(plan.steps[0].label, "Brave Search · Team");

    state
        .db
        .set_org_web_search_defaults(
            DEFAULT_ORG_ID,
            &WebSearchDefaults {
                automatic_order: vec![other.to_string(), "free".into()],
                results_per_search: 4,
                blocked_domains: vec!["pinterest.com".into()],
                ..WebSearchDefaults::default()
            },
        )
        .await
        .unwrap();
    let plan = preview(
        &state,
        OrgRole::Member,
        json!({"config": {"provider": "org_default"}}),
    )
    .await
    .unwrap();
    assert_eq!(
        step_ids(&plan),
        [
            format!("account:{other}"),
            format!("account:{team}"),
            "free".into()
        ]
    );
    assert_eq!(plan.limit, 4);
    assert_eq!(plan.org_blocked_domains, ["pinterest.com"]);

    let plan = preview(
        &state,
        OrgRole::Member,
        json!({"config": {"provider": "native", "fallback": "none"}}),
    )
    .await
    .unwrap();
    assert_eq!(step_ids(&plan), ["native"]);
    assert!(!plan.steps[0].available);
    assert!(
        plan.steps[0]
            .note
            .as_deref()
            .unwrap()
            .contains("no search of its own")
    );

    let plan = preview(
        &state,
        OrgRole::Member,
        json!({"config": {"provider": "brave"}}),
    )
    .await
    .unwrap();
    assert_eq!(step_ids(&plan), ["brave", "free"]);
    assert!(
        plan.steps[0].available,
        "falls back to the org's Brave account"
    );
}

#[tokio::test]
async fn preview_rejects_bad_input_and_member_test_queries() {
    let (state, _, _) = fixture().await;
    let error = preview(
        &state,
        OrgRole::Member,
        json!({"config": {"provider": "bing"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    let error = preview(
        &state,
        OrgRole::Member,
        json!({"model_id": "model_01933b5a00007000800000000000030b"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    let error = preview(
        &state,
        OrgRole::Member,
        json!({"test": {"query": "tokio", "provider": "free"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    for test in [
        json!({"query": " ", "provider": "free"}),
        json!({"query": "tokio", "provider": "bing"}),
        json!({"query": "tokio", "provider": format!("account:{}", Uuid::now_v7())}),
    ] {
        let error = preview(&state, OrgRole::Admin, json!({ "test": test }))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST, "{test}");
    }
}
