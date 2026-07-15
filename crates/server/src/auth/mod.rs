// Authentication module
// Decision: Modular auth with support for multiple providers
// Decision: Cookie-based sessions for UI, personal access tokens for programmatic access
// Decision: Pluggable auth backend trait for external providers (SaaS)

pub mod audit;
pub mod backend;
pub mod builtin;
pub mod caller_resolution;
pub mod cli_auth;
pub mod config;
pub mod jwt;
pub mod mcp_oauth;
pub mod middleware;
pub mod oauth;
pub mod personal_access_token;
pub mod personal_access_token_routes;
pub mod rate_limit;
pub mod routes;
pub mod share_token;

pub use backend::AuthBackend;
pub use builtin::BuiltinAuthBackend;
pub use cli_auth::CliAuthState;
pub use config::AuthConfig;
pub use mcp_oauth::McpOAuthState;
pub use middleware::{
    AuthError, AuthMethod, AuthState, AuthUser, OrgAdmin, OrgContext, OrgOwner, PlatformUser,
    ResolvedOrg,
};
pub use personal_access_token_routes::{PersonalAccessTokenState, personal_access_token_routes};
pub use routes::routes;
