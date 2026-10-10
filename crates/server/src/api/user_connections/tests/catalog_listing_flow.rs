//! A personal OAuth connect to a catalog server lists it in My MCP servers
//! (knowledge/integrations/user-mcp-servers.md, D8).

use super::*;
use crate::domains::mcp_servers::user_servers::UserMcpServers;

/// The fixture's member and the default end-user virtual user a console
/// sign-in resolves to.
async fn person_fixture() -> (AppState, ResolvedOrg, Uuid, Uuid) {
    let (state, org, server_id, _, user_id) = identity_oauth_fixture(true).await;
    let person = state
        .db
        .default_virtual_user(org.org_id, user_id)
        .await
        .unwrap()
        .id
        .uuid();
    (state, org, server_id, person)
}

fn me(org: &ResolvedOrg, person: Uuid) -> ConnectionUser {
    ConnectionUser {
        id: person,
        management_user_id: org.user_id.unwrap(),
        org_id: org.org_id,
    }
}

async fn connect(
    state: &AppState,
    org: &ResolvedOrg,
    person: Uuid,
    server_id: Uuid,
    mode: Option<&str>,
) {
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = authorize_connection(
        State(state.clone()),
        org.clone(),
        me(org, person),
        CookieJar::new(),
        Path(provider.clone()),
        Query(OAuthAuthorizeQuery {
            return_to: Some("/settings/agent-experience".to_string()),
            mode: mode.map(ToOwned::to_owned),
            session_id: None,
            agent_id: None,
            popup: None,
        }),
    )
    .await
    .unwrap();
    let pending = pending_state(&jar, &provider);
    let (_, redirect) = connection_oauth_callback(
        State(state.clone()),
        Ok(org.clone()),
        jar,
        Path(provider),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some(pending.state),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap();
    assert_connected(redirect);
}

fn assert_connected(redirect: Redirect) {
    let response = redirect.into_response();
    let target = response
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(target.contains("connected="), "{target}");
}

fn servers<'a>(state: &'a AppState, org: &ResolvedOrg, person: Uuid) -> UserMcpServers<'a> {
    UserMcpServers {
        db: &state.db,
        encryption: state.encryption.as_deref(),
        org_id: org.org_id,
        owner: person,
    }
}

#[tokio::test]
async fn a_personal_connect_lists_the_catalog_server_once() {
    let (state, org, server_id, person) = person_fixture().await;

    connect(&state, &org, person, server_id, None).await;
    connect(&state, &org, person, server_id, Some("virtual_user")).await;

    let listed = servers(&state, &org, person).list().await.unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].name, "linear");
    assert_eq!(listed[0].catalog_name.as_deref(), Some("linear"));
    assert_eq!(
        listed[0].connection.status,
        crate::domains::mcp_servers::user_servers::UserMcpConnectionStatus::Connected
    );
}

#[tokio::test]
async fn a_chat_only_session_connect_lists_nothing() {
    let (state, org, server_id, person) = person_fixture().await;
    let session_id = state.db.create_test_session().await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = authorize_connection(
        State(state.clone()),
        org.clone(),
        me(&org, person),
        CookieJar::new(),
        Path(provider.clone()),
        Query(OAuthAuthorizeQuery {
            return_to: Some("/chat".to_string()),
            mode: Some("session".to_string()),
            session_id: Some(session_id.to_string()),
            agent_id: None,
            popup: None,
        }),
    )
    .await
    .unwrap();
    let pending = pending_state(&jar, &provider);
    let (_, redirect) = connection_oauth_callback(
        State(state.clone()),
        Ok(org.clone()),
        jar,
        Path(provider),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some(pending.state),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap();
    assert_connected(redirect);

    assert!(
        servers(&state, &org, person)
            .list()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_failed_listing_does_not_break_the_connect() {
    let (state, org, server_id, person) = person_fixture().await;
    // A full list cannot take the server; the grant is still written.
    let mine = servers(&state, &org, person);
    for n in 0..crate::domains::mcp_servers::user_servers::MAX_USER_MCP_SERVERS {
        mine.add(
            crate::domains::mcp_servers::user_servers::AddUserMcpServerRequest {
                name: Some(format!("s{n}")),
                url: Some(format!("https://s{n}.example.com/mcp")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    connect(&state, &org, person, server_id, None).await;

    assert!(
        state
            .db
            .get_user_connection(person, &mcp_oauth_provider_id_for_uuid(server_id))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        mine.list()
            .await
            .unwrap()
            .iter()
            .all(|server| server.catalog_name.is_none())
    );
}
