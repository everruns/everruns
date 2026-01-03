// Authentication module
// Decision: Modular auth with support for multiple providers
// Decision: Cookie-based sessions for UI, API keys for programmatic access

pub mod api_key;
pub mod config;
pub mod jwt;
pub mod middleware;
pub mod oauth;
pub mod routes;

pub use config::AuthConfig;
pub use middleware::AuthState;
pub use routes::routes;
