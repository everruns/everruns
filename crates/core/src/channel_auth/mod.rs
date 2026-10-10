//! Credential verification for agent endpoints: OIDC/JWKS, OAuth 2.0
//! introspection, claim requirements, shared secrets, HTTP Basic and mTLS.
//!
//! Decisions:
//! - One verifier for every host. The everruns server's public channels and a
//!   serve app's agent routes both check credentials here, against the same
//!   [`ChannelAuthConfig`] shape, so a method list moves between hosts.
//! - No storage. Reading channel rows, password hashes and keys stays with the
//!   host; HTTP Basic takes the host's password check
//!   ([`ChannelAuthVerifier::with_password_check`]).
//! - Behind the `channel-auth` feature: it fetches discovery documents and key
//!   sets over the network, which the portable kernel never does. Every fetch
//!   goes through a client pinned to addresses validated against the SSRF
//!   blocklist, without redirects.
//! - The verifier remains provider-shaped so the same runtime can later read
//!   org-level reusable providers without changing ingress behavior.

mod claims;
mod config;
mod verifier;

#[cfg(test)]
pub(crate) mod test_keys;
#[cfg(test)]
mod tests;

pub use claims::{constant_time_eq, extract_bearer, verify_agentid_claims};
pub use config::{
    AGENTID_ISSUER, AGENTID_PROVIDER, ChannelAuthConfig, ChannelAuthMode,
    ChannelAuthProviderConfig, ChannelAuthRequirements, GOOGLE_ISSUER, OIDC_PROVIDER,
};
/// Key set types the verifier fetches and accepts.
pub use jsonwebtoken::jwk::JwkSet;
pub use verifier::{ChannelAuthVerifier, PasswordCheck};

/// Who a token proved the caller to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAuthPrincipal {
    /// Virtual-user binding provider: `agentid` or `oidc`.
    pub provider: String,
    pub issuer: String,
    pub subject: String,
    /// Identity namespace proven by the configured verifier, not by token claims.
    pub identity_realm: String,
}

/// Secrets a host keeps outside the [`ChannelAuthConfig`] for the
/// `shared_secret` and `api_key` modes.
#[derive(Debug, Clone, Copy, Default)]
pub struct LegacyChannelAuth<'a> {
    pub shared_secret: Option<&'a str>,
    pub api_key: Option<&'a str>,
}

/// Why a request was not let in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAuthError {
    /// No credential, or one that does not verify.
    Unauthorized,
    /// The endpoint's own config cannot verify anything.
    Misconfigured,
    /// The identity provider could not be reached or answered badly.
    ProviderUnavailable,
}

impl std::fmt::Display for ChannelAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unauthorized => "unauthorized",
            Self::Misconfigured => "authentication is misconfigured",
            Self::ProviderUnavailable => "identity provider unavailable",
        })
    }
}

impl std::error::Error for ChannelAuthError {}
