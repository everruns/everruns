// Authentication module
// Decision: Modular auth with support for multiple providers
// Decision: Cookie-based sessions for UI, API keys for programmatic access
// Decision: Pluggable auth backend trait for external providers (SaaS)

pub mod api_key;
pub mod backend;
pub mod builtin;
pub mod config;
pub mod jwt;
pub mod middleware;
pub mod oauth;
pub mod rate_limit;
pub mod routes;

pub use backend::AuthBackend;
pub use builtin::BuiltinAuthBackend;
pub use config::AuthConfig;
pub use middleware::{AuthError, AuthMethod, AuthState, AuthUser, OrgContext, ResolvedOrg};
pub use rate_limit::{AuthRateLimitConfig, AuthRateLimiter};
pub use routes::routes;
