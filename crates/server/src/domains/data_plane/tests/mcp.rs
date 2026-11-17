//! An agent's MCP servers on its org's data plane (`mcp.rs`), on a real
//! database.

use everruns_contracts::typed_id::{AgentId, VirtualUserId};
use everruns_core::mcp::oauth::TokenSet;

use super::*;
use crate::domains::agent_definitions;
use crate::domains::data_plane::mcp::{binding_name, refreshed};
use crate::domains::data_plane::providers::sealed_key;
use crate::domains::mcp_servers::McpServerOAuthSettings;
use crate::oauth_client::{McpRefreshGrant, mcp_oauth_config_hash};

const CRM_URL: &str = "https://crm.example/mcp";
const TOKEN_URL: &str = "https://crm.example/oauth/token";

struct Agent {
    key: String,
    id: AgentId,
    identity: VirtualUserId,
    provider: String,
    config_hash: String,
}

/// An agent with a literal server (`docs`, a credential header) and a
/// catalog preset it uses as its service identity (`crm`), authorized.
async fn agent(cp: &ControlPlane, encryption: &crate::storage::EncryptionService) -> Agent {
    let harness = harness(&cp.db, ORG).await;
    let catalog = format!("crm-{}", uuid::Uuid::new_v4().simple());
    let server_id = uuid::Uuid::now_v7();
    let oauth = json!({ "token_endpoint": TOKEN_URL, "client_id": "client-1" });
    cp.db
        .create_mcp_server_with_id(
            ORG,
            server_id,
            crate::storage::CreateMcpServerRow {
                name: catalog.clone(),
                description: None,
                url: CRM_URL.into(),
                transport_type: "http".into(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(json!({ "auth_mode": "o_auth", "oauth": oauth })),
            },
        )
        .await
        .unwrap();
    let identity = VirtualUserId::from_uuid(uuid::Uuid::now_v7());
    cp.db
        .create_virtual_user(crate::storage::CreateVirtualUserRow {
            org_id: ORG,
            id: identity,
            usage: "service".into(),
            name: "Agent identity".into(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
    // A public id that carries the row's id, as the console's creates do.
    let agent_id = AgentId::new();
    let row = cp
        .db
        .create_agent_with_id(
            ORG,
            agent_id,
            crate::storage::CreateAgentRow {
                completion_check: Default::default(),
                localizations: Default::default(),
                public_id: agent_id.to_string(),
                name: format!("crm-agent-{}", uuid::Uuid::new_v4().simple()),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: "Help.".into(),
                default_model_id: None,
                harness_id: harness,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({
                    "docs": {
                        "url": "https://docs.example/mcp",
                        "headers": { "Authorization": "Bearer docs-key", "X-Team": "blue" },
                    },
                    "crm": { "use": format!("catalog:{catalog}"), "actsAs": "service" },
                }),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                max_output_tokens: None,
                communication: Default::default(),
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap()
        .unwrap();
    cp.db
        .set_virtual_user_id(ORG, row.id, identity)
        .await
        .unwrap();
    let provider = format!("mcp_oauth_{server_id}");
    let settings: McpServerOAuthSettings = serde_json::from_value(oauth).unwrap();
    let config_hash = mcp_oauth_config_hash(CRM_URL, &settings);
    let grant = McpRefreshGrant {
        refresh_token: "refresh-0".into(),
        resource: CRM_URL.into(),
        config_hash: config_hash.clone(),
    };
    cp.db
        .upsert_virtual_user_connection(crate::storage::CreateVirtualUserConnectionRow {
            virtual_user_id: identity,
            provider: provider.clone(),
            connection_type: "oauth".into(),
            provider_user_id: None,
            provider_username: Some("the-agent".into()),
            access_token_encrypted: Some(encryption.encrypt_string("access-0").unwrap()),
            refresh_token_encrypted: Some(encryption.encrypt_string(&grant.encode()).unwrap()),
            scopes: None,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    let key = cp
        .db
        .engine_agent_key(ORG, harness.uuid(), Some(row.id.uuid()))
        .await
        .unwrap();
    Agent {
        key,
        id: row.id,
        identity,
        provider,
        config_hash,
    }
}

/// Publish `key`'s current platform records to the data plane, as the
/// publisher does, with this control plane.
async fn publish(cp: &ControlPlane, key: &str) -> Value {
    let target = cp.db.engine_agent_key_target(key).await.unwrap().unwrap();
    let layers = agent_definitions::resolve(
        &cp.db,
        ORG,
        everruns_contracts::typed_id::HarnessId::from_uuid(target.harness_id),
        target.agent_id.map(AgentId::from_uuid),
    )
    .await
    .unwrap();
    let declaration = agent_definitions::compile(&layers).unwrap();
    let digest = agent_definitions::digest(&declaration).unwrap();
    let revision = cp
        .db
        .publish_engine_agent(key, &serde_json::to_value(&declaration).unwrap(), &digest)
        .await
        .unwrap();
    agent_published(cp, key, ORG, &revision).await.unwrap();
    let body = cp
        .db
        .dp_item(ORG, "agent", key)
        .await
        .unwrap()
        .unwrap()
        .body
        .unwrap();
    match ConfigChange::parse(&body) {
        Some(ConfigChange::Agent(agent)) => agent.package,
        other => panic!("{other:?}"),
    }
}

async fn revision(cp: &ControlPlane, kind: &str, id: &str) -> i64 {
    cp.db
        .dp_item(ORG, kind, id)
        .await
        .unwrap()
        .unwrap()
        .revision
}

async fn tokens(cp: &ControlPlane, id: &str) -> TokenSet {
    serde_json::from_slice(&sealed_key(cp, ORG, id).await.unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn an_agents_mcp_credentials_travel_sealed_and_its_grant_comes_back_refreshed() {
    let (cp, _, _) = control_plane().await;
    let encryption = test_encryption();
    let agent = agent(&cp, &encryption).await;
    let package = publish(&cp, &agent.key).await;
    let servers = &package["mcpServers"];

    // The literal credential is a secret of the org's, named for this agent.
    let binding = binding_name(&agent.key, "docs", "Authorization");
    assert_eq!(
        servers["docs"]["headers"]["Authorization"],
        format!("${{{binding}}}"),
        "{package}"
    );
    assert_eq!(servers["docs"]["headers"]["X-Team"], "blue");
    let secret = dp::mcp_binding_secret(&binding);
    assert_eq!(
        sealed_key(&cp, ORG, &secret).await.unwrap().as_deref(),
        Some(&b"Bearer docs-key"[..])
    );
    assert!(!package.to_string().contains("docs-key"), "{package}");

    // The preset is its catalog server, acting through the agent's grant.
    let crm = &servers["crm"];
    assert_eq!(crm["url"], CRM_URL);
    assert!(
        crm.get("use").is_none() && crm.get("actsAs").is_none(),
        "{crm}"
    );
    let id = crm["oauth_provider_id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("mcp_oauth:"), "{id}");
    let connection = cp
        .db
        .dp_item(ORG, "connection", &id)
        .await
        .unwrap()
        .unwrap();
    match ConfigChange::parse(&connection.body.unwrap()) {
        Some(ConfigChange::Connection(c)) => {
            assert_eq!(c.kind, dp::MCP_OAUTH);
            assert_eq!(c.refresh_owner, dp::RefreshOwner::DataPlane);
        }
        other => panic!("{other:?}"),
    }
    let held = tokens(&cp, &id).await;
    assert_eq!(held.access_token, "access-0");
    assert_eq!(held.refresh_token.as_deref(), Some("refresh-0"));
    assert_eq!(held.resource.as_deref(), Some(CRM_URL));
    assert_eq!(held.token_endpoint.as_deref(), Some(TOKEN_URL));
    assert_eq!(held.client_id.as_deref(), Some("client-1"));

    // Publishing again changes nothing.
    let (secret_rev, grant_rev) = (
        revision(&cp, "secret", &secret).await,
        revision(&cp, "secret", &id).await,
    );
    publish(&cp, &agent.key).await;
    assert_eq!(revision(&cp, "secret", &secret).await, secret_rev);
    assert_eq!(revision(&cp, "secret", &id).await, grant_rev);

    // The data plane refreshes the grant: the platform's row gets the new
    // tokens, and the next publish does not seal the spent ones again.
    let org = cp.db.dp_org(ORG).await.unwrap().unwrap();
    let key = cp.data_key(&org).unwrap();
    let mut fresh = held.clone();
    fresh.access_token = "access-1".into();
    fresh.refresh_token = Some("refresh-1".into());
    let sealed = seal::seal(
        &key,
        &org.data_key_id,
        &org_public(ORG),
        &id,
        grant_rev as u64 + 1,
        &serde_json::to_vec(&fresh).unwrap(),
        seal::random_nonce(),
    )
    .unwrap();
    refreshed(&cp, ORG, &sealed).await.unwrap();
    cp.db
        .dp_put_item(
            ORG,
            "secret",
            &id,
            grant_rev + 1,
            Some(&ConfigChange::Secret(sealed.clone()).to_value()),
        )
        .await
        .unwrap();
    let row = cp
        .db
        .get_virtual_user_connection(agent.identity, &agent.provider)
        .await
        .unwrap()
        .unwrap();
    let decrypt =
        |v: &Option<Vec<u8>>| encryption.decrypt_to_string(v.as_deref().unwrap()).unwrap();
    assert_eq!(decrypt(&row.access_token_encrypted), "access-1");
    let grant = McpRefreshGrant::decode(&decrypt(&row.refresh_token_encrypted), &agent.config_hash)
        .expect("the stored grant is bound to the server's configuration");
    assert_eq!(grant.refresh_token, "refresh-1");
    publish(&cp, &agent.key).await;
    assert_eq!(revision(&cp, "secret", &id).await, grant_rev + 1);
    assert_eq!(tokens(&cp, &id).await.access_token, "access-1");

    // A replayed refresh is not stored twice, and a refresh over a grant the
    // person authorized again is not stored at all.
    refreshed(&cp, ORG, &sealed).await.unwrap();
    let grants = crate::storage::DbConnectionResolver::new(
        cp.db.as_ref().clone(),
        encryption.as_ref().clone(),
        None,
        crate::domains::mcp_servers::McpServerService::new(cp.db.clone(), cp.encryption.clone())
            .egress_service(),
    );
    let mut stale = grants
        .agent_service_mcp_grant(ORG, agent.id, &agent.provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stale.tokens.refresh_token.as_deref(), Some("refresh-1"));
    stale.tokens.access_token = "access-2".into();
    assert!(
        !grants
            .store_data_plane_refresh(&stale, "access-0")
            .await
            .unwrap()
    );
    let row = cp
        .db
        .get_virtual_user_connection(agent.identity, &agent.provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decrypt(&row.access_token_encrypted), "access-1");
}
