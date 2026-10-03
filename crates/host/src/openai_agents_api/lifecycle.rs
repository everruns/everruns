//! Lifecycle of provider-held state (EVE-1126): which provider failures end a
//! turn with a stable user-visible code, and how a provider session is
//! deleted.
//!
//! A provider failure that no retry fixes (rejected credentials, a preview
//! the key's project lost, a retired model, an exhausted account, a provider
//! session that no longer exists) fails the turn with a code from
//! [`everruns_contracts::user_facing_error::codes`] instead of surfacing as a
//! retried activity error. Transient failures (rate limits, 5xx, network)
//! stay errors so the durable engine retries them. Messages are written here,
//! never copied from a provider response body, which can echo part of a key.
//!
//! Design: `knowledge/execution/openai-agents-api-runtime.md#session-lifecycle`.

use everruns_contracts::user_facing_error::{codes, is_provider_quota_message};

use super::{AgentsApiClient, AgentsApiError};

/// The turn's provider session no longer exists.
pub const PROVIDER_SESSION_UNAVAILABLE: &str = codes::PROVIDER_SESSION_UNAVAILABLE;

/// A provider failure that ends the turn with a stable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleFailure {
    /// Stable user-facing code.
    pub code: &'static str,
    /// Everruns-authored explanation; never contains provider response text.
    pub message: String,
}

impl LifecycleFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Codes this module assigns; the backend reports them as is instead of
/// classifying the message text again.
pub const LIFECYCLE_CODES: &[&str] = &[
    codes::PROVIDER_SESSION_UNAVAILABLE,
    codes::PROVIDER_MISCONFIGURED,
    codes::MODEL_UNAVAILABLE,
    codes::PROVIDER_QUOTA_EXHAUSTED,
];

/// Classify a provider failure. `has_session` says whether the failed call
/// addressed an existing provider session (otherwise it was the create). A
/// `provider_session_unavailable` result is a candidate only: the caller
/// confirms that the session itself is gone before releasing it.
pub fn classify(error: &AgentsApiError, has_session: bool) -> Option<LifecycleFailure> {
    let AgentsApiError::Api { status, body } = error else {
        return None;
    };
    let status = *status;
    if is_provider_quota_message(body) {
        return Some(LifecycleFailure::new(
            codes::PROVIDER_QUOTA_EXHAUSTED,
            format!("The OpenAI account is out of credits or quota (HTTP {status})."),
        ));
    }
    if matches!(status, 400 | 403 | 404) && names_unavailable_model(body) {
        return Some(LifecycleFailure::new(
            codes::MODEL_UNAVAILABLE,
            format!(
                "OpenAI cannot serve this agent's model through the Agents API (HTTP {status})."
            ),
        ));
    }
    match status {
        401 => Some(LifecycleFailure::new(
            codes::PROVIDER_MISCONFIGURED,
            "OpenAI rejected the provider credentials (HTTP 401). Update the provider's API key.",
        )),
        403 => Some(LifecycleFailure::new(
            codes::PROVIDER_MISCONFIGURED,
            "OpenAI denied this key access to the Agents API (HTTP 403). The preview may not be enabled for the key's project.",
        )),
        404 if has_session => Some(LifecycleFailure::new(
            codes::PROVIDER_SESSION_UNAVAILABLE,
            "The OpenAI Agents API session behind this Everruns session no longer exists. The next message starts a new provider session, seeded with the recent conversation from the Everruns record.",
        )),
        404 => Some(LifecycleFailure::new(
            codes::PROVIDER_MISCONFIGURED,
            "The OpenAI Agents API is not available to this key (HTTP 404). The preview may not be enabled for the key's project.",
        )),
        _ => None,
    }
}

/// Whether a provider error body says the requested model is unavailable.
/// Only the shape is inspected; the body is never reported.
fn names_unavailable_model(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("model_not_found")
        || (lower.contains("model")
            && (lower.contains("does not exist")
                || lower.contains("not supported")
                || lower.contains("not available")
                || lower.contains("not found")))
}

/// What deleting a provider session found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderDeletion {
    /// The provider removed the session now.
    Deleted,
    /// The session was already gone (deleted earlier, expired, or not visible
    /// to these credentials). Deletion is idempotent, so this is success.
    AlreadyGone,
}

/// `DELETE /agents/sessions/{id}`, treating a missing session as deleted so a
/// retried deletion converges.
pub async fn delete_provider_session(
    client: &AgentsApiClient,
    provider_session_id: &str,
) -> Result<ProviderDeletion, AgentsApiError> {
    match client.delete_session(provider_session_id).await {
        Ok(()) => Ok(ProviderDeletion::Deleted),
        Err(AgentsApiError::Api { status: 404, .. }) => Ok(ProviderDeletion::AlreadyGone),
        Err(error) => Err(error),
    }
}

/// Stable code recorded for a failed deletion attempt, and whether retrying
/// can help. Never includes the provider response body.
pub fn deletion_failure_code(error: &AgentsApiError) -> (&'static str, bool) {
    match error {
        AgentsApiError::Api { status: 401, .. } => ("provider_auth_failed", true),
        AgentsApiError::Api { status: 403, .. } => ("provider_access_denied", true),
        AgentsApiError::Api { status: 429, .. } => ("provider_rate_limited", true),
        AgentsApiError::Api { status, .. } if *status >= 500 => ("provider_unavailable", true),
        AgentsApiError::Api { .. } => ("provider_rejected", true),
        AgentsApiError::Http(message) if message.starts_with("invalid provider id") => {
            ("invalid_provider_session_id", false)
        }
        _ => ("network_error", true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16, body: &str) -> AgentsApiError {
        AgentsApiError::Api {
            status,
            body: body.to_string(),
        }
    }

    #[test]
    fn permanent_provider_failures_get_stable_codes() {
        let cases = [
            (
                api(401, "Incorrect API key provided: sk-proj-****abcd"),
                false,
                codes::PROVIDER_MISCONFIGURED,
            ),
            (
                api(403, "agents beta not enabled"),
                true,
                codes::PROVIDER_MISCONFIGURED,
            ),
            (api(404, "not found"), false, codes::PROVIDER_MISCONFIGURED),
            (
                api(404, "No session found"),
                true,
                codes::PROVIDER_SESSION_UNAVAILABLE,
            ),
            (
                api(
                    400,
                    r#"{"error":{"code":"model_not_found","message":"The model gpt-x does not exist"}}"#,
                ),
                true,
                codes::MODEL_UNAVAILABLE,
            ),
            (
                api(429, r#"{"error":{"code":"insufficient_quota"}}"#),
                true,
                codes::PROVIDER_QUOTA_EXHAUSTED,
            ),
        ];
        for (error, has_session, code) in cases {
            let failure = classify(&error, has_session).expect("classified");
            assert_eq!(failure.code, code, "{error}");
            assert!(LIFECYCLE_CODES.contains(&failure.code));
            assert!(
                !failure.message.contains("sk-proj"),
                "provider bodies never reach the message"
            );
        }
    }

    #[test]
    fn transient_failures_stay_retryable() {
        for error in [
            api(429, "rate limit reached"),
            api(500, "oops"),
            api(503, "overloaded"),
            api(409, "conflict"),
            AgentsApiError::Http("connection reset".into()),
            AgentsApiError::StreamClosedBeforeTurnEnded,
        ] {
            assert_eq!(classify(&error, true), None, "{error}");
        }
    }

    #[test]
    fn deletion_failures_record_codes_not_bodies() {
        assert_eq!(
            deletion_failure_code(&api(401, "Incorrect API key provided: sk-proj-****abcd")),
            ("provider_auth_failed", true)
        );
        assert_eq!(
            deletion_failure_code(&api(503, "")),
            ("provider_unavailable", true)
        );
        assert_eq!(
            deletion_failure_code(&AgentsApiError::Http("invalid provider id '../x'".into())),
            ("invalid_provider_session_id", false)
        );
    }
}
