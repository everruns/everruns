use super::*;

pub(super) async fn create_oauth_test_user(server: &TestServer) -> uuid::Uuid {
    let suffix = unique_suffix();
    let user = server
        .db
        .create_user(CreateUserRow {
            email: format!("mcp-oauth-{suffix}@example.com"),
            name: "MCP OAuth Test User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("failed to create OAuth test user");
    server
        .db
        .add_organization_member(DEFAULT_ORG_ID, user.id, "member")
        .await
        .expect("failed to add OAuth test user to default org");
    user.id
}
