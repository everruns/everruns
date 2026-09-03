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
use std::sync::Arc;
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

/// A consent a human gave for one URL mode elicitation, recorded durably so the
/// tool call that follows the consent can be answered `accept`.
///
/// Consent is bound to the domain the user actually saw. A server that elicits
/// `pay.example.com`, waits for the click, then elicits `evil.example` on the
/// retry gets no reuse of the first consent: the host is compared before the
/// grant is honoured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedConsent {
    /// Host the user consented to, as shown on the consent surface.
    pub host: String,
}

/// Durable, single-use record of "a human consented to open this server's URL
/// for this tool".
///
/// A turn cannot block on a browser: the turn that hit the elicitation ends,
/// the user consents out of band, and a *later* turn re-runs the tool. The
/// consent therefore has to outlive the process that asked for it, which is why
/// this is a store rather than a channel.
#[async_trait]
pub trait ElicitationConsentStore: Send + Sync {
    /// Consume the consent recorded for `server`/`tool`, if any.
    ///
    /// THREAT[TM-TOOL-034]: taking is what keeps a consent from being replayed.
    ///
    /// Taking is destructive by contract: one consent authorises exactly one
    /// `accept`. A server that elicits again on the next call gets a fresh
    /// prompt rather than a silent replay of an old decision.
    async fn take_consent(&self, server: &str, tool: &str) -> Result<Option<GrantedConsent>>;
}

/// Handler for hosts that can pause a turn, show a consent surface, and resume:
/// it answers `accept` when — and only when — a human already consented.
///
/// The first call finds no consent and reports the elicitation through
/// [`UrlElicitationPending`], which pauses the turn and puts the URL in front of
/// the user. When they consent, the host records it and the tool runs again;
/// this time the consent is found and the server is told `accept`, so it can
/// check whether the out-of-band interaction completed.
///
/// It still never opens the URL and never invents consent — the only thing it
/// can do without a stored decision is stand down.
pub struct ConsentingUrlElicitations {
    store: Arc<dyn ElicitationConsentStore>,
}

impl ConsentingUrlElicitations {
    pub fn new(store: Arc<dyn ElicitationConsentStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl UrlElicitationHandler for ConsentingUrlElicitations {
    async fn request_url_consent(&self, elicitation: &UrlElicitation) -> Result<ElicitationAction> {
        // A store that is unreachable must not turn into an implicit "yes".
        let consent = match self
            .store
            .take_consent(&elicitation.server_name, &elicitation.tool_name)
            .await
        {
            Ok(consent) => consent,
            Err(error) => {
                tracing::warn!(
                    server = %elicitation.server_name,
                    tool = %elicitation.tool_name,
                    %error,
                    "Could not read recorded elicitation consent; asking the user again"
                );
                None
            }
        };
        let Some(consent) = consent else {
            return Ok(ElicitationAction::Cancel);
        };
        if consent.host != elicitation.host {
            tracing::warn!(
                server = %elicitation.server_name,
                tool = %elicitation.tool_name,
                consented_host = %consent.host,
                requested_host = %elicitation.host,
                "MCP server elicited a different domain than the user consented to; \
                 asking again"
            );
            return Ok(ElicitationAction::Cancel);
        }
        Ok(ElicitationAction::Accept)
    }
}

/// How long a recorded consent stays usable.
///
/// Long enough for a real out-of-band interaction (sign in, approve, pay),
/// short enough that a consent cannot be replayed against an elicitation the
/// user has forgotten about.
pub const CONSENT_TTL: chrono::Duration = chrono::Duration::minutes(30);

/// The durable form of a consent: what was consented to, by which pairing, and
/// until when.
///
/// Written by whoever collects the decision (the API that serves the consent
/// surface) and read by the MCP client on the retry, so it is serialized rather
/// than passed in memory. It holds no secret and no `requestState` — the value
/// the server wants never travels this path, and each retry carries its own
/// round's state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredConsent {
    /// Logical MCP server the consent is for. Re-checked on read so a key
    /// collision fails closed instead of granting the wrong server an accept.
    pub server: String,
    /// MCP tool the consent is for. Re-checked on read for the same reason.
    pub tool: String,
    /// Domain the user actually saw and agreed to open.
    pub host: String,
    /// When the consent stops being usable.
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl StoredConsent {
    pub fn new(server: &str, tool: &str, host: &str, now: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            server: server.to_string(),
            tool: tool.to_string(),
            host: host.to_string(),
            expires_at: now + CONSENT_TTL,
        }
    }

    /// Honour the record only for the pairing it names and only while fresh.
    pub fn grant_for(
        &self,
        server: &str,
        tool: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<GrantedConsent> {
        if self.server != server || self.tool != tool || self.expires_at <= now {
            return None;
        }
        Some(GrantedConsent {
            host: self.host.clone(),
        })
    }
}

/// Session-storage key a consent for `server`/`tool` is recorded under.
///
/// Session storage keys are flat strings that a user can see, so the key is
/// readable rather than hashed; characters that would make it ambiguous are
/// folded to `_`. Folding can in principle collide, which is why
/// [`StoredConsent`] repeats the pairing and [`StoredConsent::grant_for`]
/// re-checks it.
pub fn consent_storage_key(server: &str, tool: &str) -> String {
    fn fold(part: &str) -> String {
        part.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
    format!("mcp/elicitation-consent/{}/{}", fold(server), fold(tool))
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
/// THREAT[TM-TOOL-033]: the URL comes from a third-party MCP server and is put
/// in front of a person. Validate the scheme here, before any handler sees it,
/// and never fetch it.
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

    /// A consent store that hands out one prepared record, and remembers that
    /// it was taken.
    struct OneShotConsents {
        record: std::sync::Mutex<Option<StoredConsent>>,
    }

    impl OneShotConsents {
        fn holding(record: StoredConsent) -> Self {
            Self {
                record: std::sync::Mutex::new(Some(record)),
            }
        }

        fn empty() -> Self {
            Self {
                record: std::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl ElicitationConsentStore for OneShotConsents {
        async fn take_consent(&self, server: &str, tool: &str) -> Result<Option<GrantedConsent>> {
            let taken = self.record.lock().expect("lock").take();
            Ok(taken.and_then(|record| record.grant_for(server, tool, chrono::Utc::now())))
        }
    }

    fn elicitation(host: &str) -> UrlElicitation {
        UrlElicitation {
            server_name: "billing".to_string(),
            tool_name: "charge".to_string(),
            key: "pay".to_string(),
            message: "Complete the payment".to_string(),
            url: format!("https://{host}/pay/1"),
            host: host.to_string(),
            punycode: false,
        }
    }

    #[tokio::test]
    async fn accepts_only_once_a_human_has_consented() {
        let handler = ConsentingUrlElicitations::new(Arc::new(OneShotConsents::empty()));
        assert_eq!(
            handler
                .request_url_consent(&elicitation("pay.example.com"))
                .await
                .expect("handled"),
            ElicitationAction::Cancel,
            "no recorded consent must never become an implicit accept"
        );

        let handler = ConsentingUrlElicitations::new(Arc::new(OneShotConsents::holding(
            StoredConsent::new("billing", "charge", "pay.example.com", chrono::Utc::now()),
        )));
        assert_eq!(
            handler
                .request_url_consent(&elicitation("pay.example.com"))
                .await
                .expect("handled"),
            ElicitationAction::Accept
        );
    }

    #[tokio::test]
    async fn refuses_to_reuse_a_consent_for_another_domain() {
        let handler = ConsentingUrlElicitations::new(Arc::new(OneShotConsents::holding(
            StoredConsent::new("billing", "charge", "pay.example.com", chrono::Utc::now()),
        )));
        // The user consented to pay.example.com; the retry elicits somewhere
        // else. That is the swap the consent record exists to catch.
        assert_eq!(
            handler
                .request_url_consent(&elicitation("evil.example"))
                .await
                .expect("handled"),
            ElicitationAction::Cancel
        );
    }

    #[tokio::test]
    async fn consent_is_single_use() {
        let store = Arc::new(OneShotConsents::holding(StoredConsent::new(
            "billing",
            "charge",
            "pay.example.com",
            chrono::Utc::now(),
        )));
        let handler = ConsentingUrlElicitations::new(store);
        let first = handler
            .request_url_consent(&elicitation("pay.example.com"))
            .await
            .expect("handled");
        let second = handler
            .request_url_consent(&elicitation("pay.example.com"))
            .await
            .expect("handled");
        assert_eq!(first, ElicitationAction::Accept);
        assert_eq!(
            second,
            ElicitationAction::Cancel,
            "one consent authorises exactly one accept"
        );
    }

    #[test]
    fn expired_or_mismatched_records_grant_nothing() {
        let now = chrono::Utc::now();
        let record = StoredConsent::new("billing", "charge", "pay.example.com", now);
        assert!(record.grant_for("billing", "charge", now).is_some());
        assert!(record.grant_for("other", "charge", now).is_none());
        assert!(record.grant_for("billing", "refund", now).is_none());
        assert!(
            record
                .grant_for("billing", "charge", now + CONSENT_TTL)
                .is_none()
        );
    }

    #[test]
    fn consent_keys_stay_readable_and_scoped() {
        assert_eq!(
            consent_storage_key("billing", "charge"),
            "mcp/elicitation-consent/billing/charge"
        );
        // Separators inside a name must not invent extra key segments.
        assert_eq!(
            consent_storage_key("acme/billing", "charge"),
            "mcp/elicitation-consent/acme_billing/charge"
        );
    }
}
