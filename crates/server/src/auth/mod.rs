// Authentication module
// Decision: Modular auth with support for multiple providers
// Decision: Cookie-based sessions for UI, API keys for programmatic access
// Decision: Pluggable auth backend trait for external providers (SaaS)

pub mod api_key;
pub mod audit;
pub mod backend;
pub mod builtin;
pub mod cli_auth;
pub mod config;
pub mod jwt;
pub mod mcp_oauth;
pub mod middleware;
pub mod oauth;
pub mod rate_limit;
pub mod routes;

pub use backend::AuthBackend;
pub use builtin::BuiltinAuthBackend;
pub use cli_auth::CliAuthState;
pub use config::AuthConfig;
pub use mcp_oauth::McpOAuthState;
pub use middleware::{
    AuthError, AuthMethod, AuthState, AuthUser, OrgAdmin, OrgContext, OrgOwner, ResolvedOrg,
};
pub use routes::routes;
