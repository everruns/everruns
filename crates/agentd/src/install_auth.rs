//! Who may call an install's API (agentd design, "Auth"): services only.
//! There is no end-user, browser or anonymous access; only `serve --dev`
//! on a loopback address skips auth.
//!
//! Decisions:
//! - API keys are `id:secret` pairs (`AGENTD_API_KEYS`, or a file with one
//!   per line). Only their SHA-256 is kept in memory, compared in constant
//!   time. The id names the caller: it is what the one-controller rule
//!   records, and what logs show; the secret never is.
//! - A JWT signed by a key in the controller's JWKS (`AGENTD_JWKS_URL`)
//!   counts too, as the hosted control plane's org tokens do: it needs the
//!   `admin` scope, and its `sub` names the caller.
//! - A client certificate the install's CA signed (`install_tls.rs`) is
//!   checked by TLS before any request reaches here; with one, a request
//!   needs no bearer, and the certificate's subject names the caller.
//! - `/health` stays open, so a load balancer can probe without a key.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sha2::Digest;

use crate::install::Caller;

#[derive(Clone, Default)]
pub struct Auth {
    /// No checks: `serve --dev` on loopback only.
    pub open: bool,
    keys: Vec<(String, [u8; 32])>,
    tokens: Option<Arc<crate::org_token::OrgTokens>>,
}

/// The caller a verified client certificate names (`install_tls.rs`).
#[derive(Debug, Clone)]
pub struct ClientCert(pub String);

impl Auth {
    /// No auth at all (`serve --dev`).
    pub fn open() -> Self {
        Self {
            open: true,
            ..Self::default()
        }
    }

    /// Keys from `id:secret` lines (blank lines and `#` comments skipped).
    pub fn with_keys(mut self, text: &str) -> anyhow::Result<Self> {
        for line in text.split([',', '\n']).map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (id, secret) = line
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("an API key is `id:secret`"))?;
            anyhow::ensure!(
                crate::install::valid_name(id),
                "API key id {id:?}: letters, digits, `-`, `_`, `.`"
            );
            anyhow::ensure!(
                secret.len() >= 24,
                "API key {id}: the secret needs at least 24 characters"
            );
            self.keys
                .push((id.to_owned(), sha2::Sha256::digest(secret).into()));
        }
        Ok(self)
    }

    /// Accept JWTs signed by the keys at `jwks_url`.
    pub fn with_jwks(mut self, jwks_url: String) -> Self {
        self.tokens = Some(Arc::new(crate::org_token::OrgTokens::new(
            Some(jwks_url),
            None,
        )));
        self
    }

    /// Whether any credential is configured.
    pub fn has_credentials(&self) -> bool {
        !self.keys.is_empty() || self.tokens.is_some()
    }

    /// The caller `bearer` names, if it is one of ours.
    pub async fn caller(&self, bearer: &str) -> Option<String> {
        let sent: [u8; 32] = sha2::Sha256::digest(bearer).into();
        let mut found = None;
        // Every key is compared, so the time taken says nothing of which.
        for (id, hash) in &self.keys {
            if crate::node_control::same_secret(&sent, hash) {
                found = Some(id.clone());
            }
        }
        if found.is_some() {
            return found;
        }
        let tokens = self.tokens.as_ref()?;
        let claims = tokens.verify(bearer).await.ok()?;
        use crate::org_token::{ADMIN, OrgTokenClaimsExt};
        claims.has(ADMIN).then(|| format!("jwt:{}", claims.sub))
    }
}

/// The layer in front of an install's API.
pub async fn check(
    axum::extract::State(auth): axum::extract::State<Auth>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }
    if auth.open {
        request.extensions_mut().insert(Caller("dev".into()));
        return next.run(request).await;
    }
    if let Some(ClientCert(subject)) = request.extensions().get::<ClientCert>().cloned() {
        request
            .extensions_mut()
            .insert(Caller(format!("cert:{subject}")));
        return next.run(request).await;
    }
    let Some(bearer) = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.trim().to_owned())
    else {
        return (
            StatusCode::UNAUTHORIZED,
            "send an API key: Authorization: Bearer <key>",
        )
            .into_response();
    };
    match auth.caller(&bearer).await {
        Some(caller) => {
            request.extensions_mut().insert(Caller(caller));
            next.run(request).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            "the API key is not one of this install's",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_key_names_its_caller_and_nothing_else_passes() {
        let auth = Auth::default()
            .with_keys("app:0123456789abcdef01234567\n# a comment\nops:zyxwvutsrqponmlkjihgfedcba")
            .unwrap();
        assert_eq!(
            auth.caller("0123456789abcdef01234567").await.as_deref(),
            Some("app")
        );
        assert_eq!(
            auth.caller("zyxwvutsrqponmlkjihgfedcba").await.as_deref(),
            Some("ops")
        );
        assert_eq!(auth.caller("app:0123456789abcdef01234567").await, None);
        assert_eq!(auth.caller("").await, None);
    }

    #[test]
    fn short_or_malformed_keys_are_refused() {
        assert!(Auth::default().with_keys("app:short").is_err());
        assert!(
            Auth::default()
                .with_keys("no-colon-here-at-all-long-enough")
                .is_err()
        );
        assert!(
            Auth::default()
                .with_keys("bad id:0123456789abcdef01234567")
                .is_err()
        );
    }
}
