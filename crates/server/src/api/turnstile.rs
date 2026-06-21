// Cloudflare Turnstile verification for the Public Chat channel.
//
// Turnstile is the bot-mitigation challenge enforced on anonymous Public Chat
// access (see `specs/public-chat.md`). The client widget produces a token that
// is verified server-side here before any session is created or any turn runs.
// Signed-in visitors bypass the challenge and never reach this verifier.
//
// The siteverify URL is a fixed, trusted Cloudflare endpoint and is not
// operator-configurable in production, so there is no SSRF surface to pin. The
// pure `interpret` step is unit-tested; the network call is a thin wrapper.

use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;

use serde::Deserialize;

/// Shared HTTP client for Turnstile siteverify. Built once; reused per request.
/// Falls back to a default client if the builder ever fails (it does not in
/// practice with these options).
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(VERIFY_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
});

/// Cloudflare Turnstile server-side verification endpoint.
pub const TURNSTILE_SITEVERIFY_URL: &str =
    "https://challenges.cloudflare.com/turnstile/v0/siteverify";

const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Outcome of a Turnstile verification attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnstileOutcome {
    /// The token is valid; allow the request.
    Allowed,
    /// The token is missing, malformed, expired, or rejected; deny (403).
    Rejected,
    /// The challenge service could not be reached or returned an error; the
    /// caller should fail closed but may surface a "try again" message (503).
    Unavailable,
}

/// Raw Cloudflare siteverify response. Only the fields we act on are modeled.
#[derive(Debug, Clone, Deserialize, Default)]
struct SiteverifyResponse {
    success: bool,
    #[serde(default, rename = "error-codes")]
    error_codes: Vec<String>,
}

/// Map a decoded siteverify response to an outcome. Pure and unit-tested.
fn interpret(response: &SiteverifyResponse) -> TurnstileOutcome {
    if response.success {
        TurnstileOutcome::Allowed
    } else {
        // A non-success with no error codes still means "not verified".
        // Internal/service-side error codes are treated as Unavailable so a
        // Cloudflare outage does not look identical to a forged token.
        let service_side = response
            .error_codes
            .iter()
            .any(|code| code == "internal-error" || code == "bad-gateway");
        if service_side {
            TurnstileOutcome::Unavailable
        } else {
            TurnstileOutcome::Rejected
        }
    }
}

/// Verifies Turnstile tokens against Cloudflare's siteverify endpoint.
#[derive(Clone)]
pub struct TurnstileVerifier {
    siteverify_url: String,
}

impl Default for TurnstileVerifier {
    fn default() -> Self {
        Self {
            siteverify_url: TURNSTILE_SITEVERIFY_URL.to_string(),
        }
    }
}

impl TurnstileVerifier {
    /// Verify a client token. `remote_ip` is forwarded to Cloudflare as an
    /// additional signal when available. A missing/empty token short-circuits
    /// to `Rejected` without a network call.
    pub async fn verify(
        &self,
        secret: &str,
        token: &str,
        remote_ip: Option<IpAddr>,
    ) -> TurnstileOutcome {
        if token.trim().is_empty() || secret.trim().is_empty() {
            return TurnstileOutcome::Rejected;
        }

        // The siteverify URL is fixed and trusted, so a single shared client
        // (connection pool + TLS config) is reused across requests.
        let client = &*HTTP_CLIENT;

        let mut form = vec![
            ("secret", secret.to_string()),
            ("response", token.to_string()),
        ];
        if let Some(ip) = remote_ip {
            form.push(("remoteip", ip.to_string()));
        }

        let result = client.post(&self.siteverify_url).form(&form).send().await;
        let response = match result {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(error = %err, "Turnstile siteverify request failed");
                return TurnstileOutcome::Unavailable;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Turnstile siteverify non-2xx");
            return TurnstileOutcome::Unavailable;
        }
        match response.json::<SiteverifyResponse>().await {
            Ok(decoded) => interpret(&decoded),
            Err(err) => {
                tracing::warn!(error = %err, "Turnstile siteverify decode failed");
                TurnstileOutcome::Unavailable
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(success: bool, codes: &[&str]) -> SiteverifyResponse {
        SiteverifyResponse {
            success,
            error_codes: codes.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn success_is_allowed() {
        assert_eq!(interpret(&resp(true, &[])), TurnstileOutcome::Allowed);
    }

    #[test]
    fn invalid_token_is_rejected() {
        assert_eq!(
            interpret(&resp(false, &["invalid-input-response"])),
            TurnstileOutcome::Rejected
        );
    }

    #[test]
    fn timeout_or_duplicate_is_rejected() {
        assert_eq!(
            interpret(&resp(false, &["timeout-or-duplicate"])),
            TurnstileOutcome::Rejected
        );
    }

    #[test]
    fn service_side_error_is_unavailable() {
        assert_eq!(
            interpret(&resp(false, &["internal-error"])),
            TurnstileOutcome::Unavailable
        );
    }

    #[test]
    fn empty_failure_is_rejected() {
        assert_eq!(interpret(&resp(false, &[])), TurnstileOutcome::Rejected);
    }

    #[tokio::test]
    async fn missing_token_short_circuits_to_rejected() {
        let verifier = TurnstileVerifier::default();
        assert_eq!(
            verifier.verify("secret", "", None).await,
            TurnstileOutcome::Rejected
        );
    }

    #[tokio::test]
    async fn missing_secret_short_circuits_to_rejected() {
        let verifier = TurnstileVerifier::default();
        assert_eq!(
            verifier.verify("  ", "token", None).await,
            TurnstileOutcome::Rejected
        );
    }
}
