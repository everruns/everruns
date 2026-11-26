use super::RouteAccess::{self, Never, Read};

/// `(method, path, class)` for served routes the OpenAPI document omits.
pub const UNDOCUMENTED_ROUTES: &[(&str, &str, RouteAccess)] = &[
    // Readable.
    ("GET", "/v1/platform/auth/me", Read),
    ("GET", "/v1/platform/budgets/config", Read),
    ("GET", "/v1/platform/environment-targets", Read),
    ("GET", "/v1/platform/environments", Read),
    (
        "GET",
        "/v1/platform/environments/{sandbox_template_id}",
        Read,
    ),
    ("GET", "/v1/platform/feature-flags", Read),
    ("GET", "/v1/platform/notifications/sse", Read),
    ("GET", "/v1/platform/orgs/{org}/invites", Read),
    ("GET", "/v1/platform/orgs/{org}/members", Read),
    ("GET", "/v1/platform/sessions/config", Read),
    ("GET", "/v1/platform/sessions/{session_id}/commands", Read),
    (
        "GET",
        "/v1/platform/sessions/{session_id}/environment",
        Read,
    ),
    ("GET", "/v1/platform/sessions/{session_id}/fs", Read),
    (
        "GET",
        "/v1/platform/sessions/{session_id}/fs/_/download/{*path}",
        Read,
    ),
    ("GET", "/v1/platform/sessions/{session_id}/fs/{*path}", Read),
    (
        "GET",
        "/v1/platform/sessions/{session_id}/mcp-apps/view",
        Read,
    ),
    ("GET", "/v1/platform/slack/workspaces", Read),
    ("GET", "/v1/platform/user/connections", Read),
    ("GET", "/v1/platform/user/connections/providers", Read),
    ("GET", "/v1/platform/user/preferences", Read),
    ("GET", "/v1/platform/user/preferences/{key}", Read),
    ("GET", "/v1/platform/virtual-users/config", Read),
    ("GET", "/v1/virtual-users/me/mcp-connections", Read),
    (
        "GET",
        "/v1/platform/virtual-users/{identity_id}/connections/github/repositories",
        Read,
    ),
    ("POST", "/v1/platform/sessions/{session_id}/fs/_/grep", Read),
    (
        "POST",
        "/v1/platform/sessions/{session_id}/fs/_/search",
        Read,
    ),
    ("POST", "/v1/platform/sessions/{session_id}/fs/_/stat", Read),
    // Membership, invitations, OAuth starts and callbacks, GitHub App
    // setup and Slack install.
    ("POST", "/v1/platform/orgs/{org}/invites", Never),
    (
        "DELETE",
        "/v1/platform/orgs/{org}/invites/{invite_id}",
        Never,
    ),
    ("POST", "/v1/invites/{token}/accept", Never),
    ("POST", "/v1/platform/orgs/{org}/members", Never),
    ("PATCH", "/v1/platform/orgs/{org}/members/{user_id}", Never),
    ("DELETE", "/v1/platform/orgs/{org}/members/{user_id}", Never),
    ("GET", "/v1/platform/providers/{id}/oauth/authorize", Never),
    ("GET", "/v1/providers/{id}/oauth/callback", Never),
    (
        "GET",
        "/v1/platform/user/connections/{provider}/authorize",
        Never,
    ),
    ("GET", "/v1/user/connections/{provider}/callback", Never),
    (
        "GET",
        "/v1/platform/user/connections/github/authorize",
        Never,
    ),
    ("GET", "/v1/user/connections/github/callback", Never),
    ("GET", "/v1/connection-callbacks/{provider}", Never),
    ("GET", "/v1/connection-callbacks/github", Never),
    ("GET", "/v1/github/app-manifest/callback", Never),
    ("GET", "/v1/github/apps/{app_row_id}/setup", Never),
    (
        "POST",
        "/v1/platform/virtual-users/{identity_id}/connections/github/app",
        Never,
    ),
    ("GET", "/v1/platform/slack/install", Never),
    ("POST", "/v1/platform/slack/workspaces", Never),
    ("POST", "/v1/platform/slack/workspaces/oauth", Never),
    ("GET", "/v1/slack/workspaces/oauth/callback", Never),
    (
        "POST",
        "/v1/platform/channels/{channel_id}/slack/install",
        Never,
    ),
    ("POST", "/v1/platform/e/{channel_id}/slack/install", Never),
    (
        "GET",
        "/v1/channels/{channel_id}/slack/oauth/callback",
        Never,
    ),
    ("GET", "/v1/e/{channel_id}/slack/oauth/callback", Never),
];

/// Non-GET routes classified `Read`, each reviewed to persist nothing.
pub const REVIEWED_NON_GET_READ: &[(&str, &str)] = &[
    // diff_agent_package: compares, persists nothing
    ("POST", "/v1/platform/agents/diff"),
    // validate_agent_package: validates, persists nothing
    ("POST", "/v1/platform/agents/validate"),
    // Retrieves evidence without changing collection configuration or files.
    // Embedding usage accounting and derived caches follow ordinary read billing.
    ("POST", "/v1/platform/knowledge/search"),
    // searches memory files
    ("POST", "/v1/platform/memories/{memory_id}/fs/_/grep"),
    // reads file metadata
    ("POST", "/v1/platform/memories/{memory_id}/fs/_/stat"),
    // grep_workspace_files
    ("POST", "/v1/platform/sessions/{session_id}/fs/_/grep"),
    // search_workspace_files
    ("POST", "/v1/platform/sessions/{session_id}/fs/_/search"),
    // stat_workspace_file
    ("POST", "/v1/platform/sessions/{session_id}/fs/_/stat"),
    // grep_workspace_files
    ("POST", "/v1/platform/workspaces/{workspace_id}/fs/_/grep"),
    // stat_workspace_file
    ("POST", "/v1/platform/workspaces/{workspace_id}/fs/_/stat"),
];

/// Path prefixes that are `Never` unless a row above says otherwise: the auth
/// surface (login, registration, refresh, recovery, OAuth, CLI login and
/// personal access token management). `GET /v1/platform/auth/me` is the one `Read` row.
// Published identity callbacks retain their original prefix. Neither auth
// namespace may become available to scoped tokens.
pub const NEVER_PREFIXES: &[&str] = &["/v1/platform/auth/", "/v1/auth/"];
