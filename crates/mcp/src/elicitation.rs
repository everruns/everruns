//! URL mode elicitation (MCP `2026-07-28`, introduced in `2025-11-25`).
//!
//! A server that needs a secret, a third-party authorization, or a payment must
//! not ask for it through the MCP client: the value would transit an
//! intermediary and land in model context. Instead it answers a `tools/call`
//! with an MRTR `input_required` result carrying an `elicitation/create`
//! request in `mode: "url"`, and the client asks a human to open that URL out
//! of band.
//!
//! Everruns' client therefore never *answers* the elicitation — it only decides
//! whether a human consents to opening the URL, then retries the tool call so
//! the server can check whether the out-of-band interaction completed.
//!
//! Consent is a host concern: a chat UI can render a consent card, a CLI can
//! prompt, and an unattended worker can reach nobody at all. So hosts inject a
//! [`UrlElicitationHandler`]; a host that injects none declines every
//! elicitation, and the client declares no `elicitation` capability at all,
//! which under MRTR is what stops servers from asking.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use url::Url;

/// A server's request that a human visit a URL out of band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlElicitation {
    /// Logical MCP server that asked, for "which server is asking?" UI.
    pub server_name: String,
    /// Tool call the elicitation interrupted.
    pub tool_name: String,
    /// Server-assigned key in the `inputRequests` map; the response must be
    /// returned under the same key.
    pub key: String,
    /// Human-readable reason the interaction is needed.
    pub message: String,
    /// The URL to open. Validated by [`validate_elicitation_url`] before a
    /// handler ever sees it, and never fetched by the client.
    pub url: String,
    /// Host of `url`, pre-extracted so a consent surface can highlight the
    /// domain rather than re-parsing (clients **SHOULD** highlight the domain
    /// to mitigate subdomain spoofing).
    pub host: String,
    /// Whether the host contains a Punycode label (`xn--`). Not rejected —
    /// internationalized domains are legitimate — but flagged so a consent
    /// surface can warn, as the spec requires for ambiguous URIs.
    pub punycode: bool,
}

/// What a human decided about a URL elicitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElicitationAction {
    /// Consented to opening the URL. Does *not* mean the out-of-band
    /// interaction finished — only the server knows that.
    Accept,
    /// Explicitly refused.
    Decline,
    /// Dismissed without deciding (closed the surface, timed out, no human
    /// reachable).
    Cancel,
}

impl ElicitationAction {
    /// Wire value for the `action` field of an `ElicitResult`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Decline => "decline",
            Self::Cancel => "cancel",
        }
    }
}

/// Gets human consent for a URL mode elicitation.
///
/// Implementations **MUST NOT** fetch the URL or any of its metadata, **MUST**
/// show the full URL, and **MUST NOT** open it without explicit consent. A
/// handler that cannot reach a human returns [`ElicitationAction::Cancel`]
/// promptly rather than blocking: MRTR gives the server no way to wait, and the
/// tool call is holding a turn open.
#[async_trait]
pub trait UrlElicitationHandler: Send + Sync {
    async fn request_url_consent(&self, elicitation: &UrlElicitation) -> Result<ElicitationAction>;
}

/// A URL elicitation that stopped a tool call, kept typed so the executor can
/// hand the user an actionable affordance instead of a flat error string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "MCP server '{server_name}' needs you to complete an interaction at {host} before tool \
     '{tool_name}' can run"
)]
pub struct UrlElicitationPending {
    pub server_name: String,
    pub tool_name: String,
    pub message: String,
    pub url: String,
    pub host: String,
    pub punycode: bool,
    /// What the handler answered. `Cancel` from the relay handler means "shown
    /// to the user, not yet completed"; `Decline` means they refused.
    pub action: ElicitationAction,
}

/// Handler for hosts that can show a URL to the session's user but cannot block
/// on them: the elicitation is reported through the tool result, and the user
/// re-runs the tool once they have finished.
///
/// It answers `cancel`, never `accept`: the client must not claim consent it did
/// not obtain, and it never opens the URL itself. The elicitation surfaces to
/// the user through [`UrlElicitationPending`], and the retry that follows is the
/// manual continuation the spec asks clients to provide when an out-of-band
/// interaction cannot be waited on.
pub struct RelayUrlElicitations;

#[async_trait]
impl UrlElicitationHandler for RelayUrlElicitations {
    async fn request_url_consent(
        &self,
        _elicitation: &UrlElicitation,
    ) -> Result<ElicitationAction> {
        Ok(ElicitationAction::Cancel)
    }
}

/// Handler for hosts with no human in the loop: declines everything.
///
/// Not the same as injecting no handler at all. No handler means the client
/// declares no `elicitation` capability and servers must not ask; this one is
/// for hosts that want the capability declared (e.g. a shared transport) but
/// have no reachable human for a particular run.
pub struct DeclineUrlElicitations;

#[async_trait]
impl UrlElicitationHandler for DeclineUrlElicitations {
    async fn request_url_consent(
        &self,
        _elicitation: &UrlElicitation,
    ) -> Result<ElicitationAction> {
        Ok(ElicitationAction::Decline)
    }
}

/// Validate a server-supplied elicitation URL before showing it to anyone.
///
/// Returns the URL's host and whether it carries a Punycode label. Rejects
/// anything that is not `https`, with one exception: loopback `http` for local
/// development, matching the OAuth redirect rule in [`crate::oauth`]. Schemes
/// like `javascript:`, `file:`, and `data:` are the reason this is a hard gate
/// rather than a handler concern — a consent surface must never be handed one.
pub fn validate_elicitation_url(url: &str) -> Result<(String, bool)> {
    let parsed = Url::parse(url).map_err(|e| anyhow!("elicitation URL is not a valid URL: {e}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("elicitation URL has no host"))?
        .to_string();
    let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1";
    match parsed.scheme() {
        "https" => {}
        "http" if loopback => {}
        scheme => {
            return Err(anyhow!(
                "elicitation URL must use https (got '{scheme}'); \
                 http is accepted only on loopback for local development"
            ));
        }
    }
    let punycode = host.split('.').any(|label| label.starts_with("xn--"));
    Ok((host, punycode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_and_reports_host() {
        let (host, punycode) = validate_elicitation_url("https://auth.example.com/connect?x=1")
            .expect("https accepted");
        assert_eq!(host, "auth.example.com");
        assert!(!punycode);
    }

    #[test]
    fn accepts_loopback_http_for_local_development() {
        assert!(validate_elicitation_url("http://localhost:3000/connect").is_ok());
        assert!(validate_elicitation_url("http://127.0.0.1:3000/connect").is_ok());
    }

    #[test]
    fn rejects_non_https_and_dangerous_schemes() {
        for url in [
            "http://auth.example.com/connect",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<script>",
            "not-a-url",
        ] {
            assert!(
                validate_elicitation_url(url).is_err(),
                "expected {url} to be rejected"
            );
        }
    }

    #[test]
    fn flags_punycode_hosts_without_rejecting_them() {
        let (host, punycode) =
            validate_elicitation_url("https://xn--80ak6aa92e.com/connect").expect("accepted");
        assert_eq!(host, "xn--80ak6aa92e.com");
        assert!(punycode);
    }
}
