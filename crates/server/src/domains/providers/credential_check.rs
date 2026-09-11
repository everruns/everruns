// Provider credential check — a live, read-only probe with no persistence.
//
// Decision: setup must not accept an API key the provider will reject. A bad
// key otherwise lands encrypted in the DB and only surfaces much later, on the
// first agent run, far from where the user could fix it. The probe is a
// `list_models` call with the candidate credential; no provider row is written,
// so a rejected key never reaches storage.
//
// The outcome is classified from the driver's `LlmErrorKind`, which is assigned
// at the provider boundary where the HTTP status and body are still available
// (see `everruns_provider::error`). Only `Rejected` is a hard stop for callers:
// `Unsupported` (driver exposes no discovery endpoint) and `Unreachable`
// (network failure, provider outage) say nothing about the key and must not
// block setup.

use crate::kernel_imports::{
    everruns_provider::driver_registry::DriverRegistry,
    everruns_provider::driver_registry::ProviderConfig, everruns_provider::provider::DriverId,
};
use everruns_provider::error::{AgentLoopError, LlmErrorKind};
use serde::Serialize;
use utoipa::ToSchema;

/// Outcome of checking a candidate provider credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CredentialCheckResult {
    /// The provider accepted the credential.
    Valid {
        /// Number of models the provider listed for this credential.
        models: usize,
    },
    /// The provider rejected the credential (401/403). Hard stop.
    Rejected {
        /// User-facing reason. Never carries the provider's response body.
        message: String,
    },
    /// This driver has no credential-checking endpoint (custom base URL,
    /// simulator, or a driver without model discovery).
    Unsupported,
    /// The provider could not be reached, so the credential is unproven.
    Unreachable {
        /// User-facing reason. Never carries the provider's response body.
        message: String,
    },
}

/// Probe `provider_type` with `api_key` by listing its models.
///
/// `base_url` must already have passed the same validation as provider
/// creation — this issues a real outbound request to it.
pub async fn check_credentials(
    registry: &DriverRegistry,
    provider_type: DriverId,
    api_key: String,
    base_url: Option<String>,
) -> CredentialCheckResult {
    // The simulator driver has no upstream to ask.
    if provider_type == DriverId::LlmSim {
        return CredentialCheckResult::Unsupported;
    }

    let config = ProviderConfig {
        provider: everruns_provider::runtime_provider::ProviderKey::new("credential-check"),
        provider_type,
        api_key: Some(api_key),
        base_url,
        metadata: Default::default(),
        request_options: Default::default(),
    };

    let driver = match registry.create_chat_driver(&config) {
        Ok(driver) => driver,
        Err(e) => {
            tracing::debug!(error = %e, "Credential check: no driver for provider type");
            return CredentialCheckResult::Unsupported;
        }
    };

    match driver
        .list_models(&everruns_provider::runtime_provider::ProviderEndpoint::default())
        .await
    {
        Ok(Some(models)) => CredentialCheckResult::Valid {
            models: models.len(),
        },
        Ok(None) => CredentialCheckResult::Unsupported,
        Err(e) => {
            // Detail stays server-side: the response body of a failed provider
            // call is not something to echo back to the browser.
            tracing::info!(error = %e, "Credential check failed");
            classify_failure(&e)
        }
    }
}

/// Map a driver failure onto a check outcome. Only an authentication failure
/// proves the credential itself is bad.
fn classify_failure(err: &AgentLoopError) -> CredentialCheckResult {
    match err.llm_error_kind() {
        Some(LlmErrorKind::Authentication) => CredentialCheckResult::Rejected {
            message: "The provider rejected this API key.".to_string(),
        },
        _ => CredentialCheckResult::Unreachable {
            message: "Could not reach the provider to verify this API key.".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_failure_is_rejected() {
        let err = AgentLoopError::llm_kind(LlmErrorKind::Authentication, "401");
        assert!(matches!(
            classify_failure(&err),
            CredentialCheckResult::Rejected { .. }
        ));
    }

    #[test]
    fn outage_is_unreachable_not_rejected() {
        for kind in [
            LlmErrorKind::Unavailable,
            LlmErrorKind::RateLimited,
            LlmErrorKind::Other,
        ] {
            let err = AgentLoopError::llm_kind(kind, "boom");
            assert!(
                matches!(
                    classify_failure(&err),
                    CredentialCheckResult::Unreachable { .. }
                ),
                "{kind:?} must not read as a bad key"
            );
        }
    }

    #[test]
    fn non_llm_failure_is_unreachable() {
        let err = AgentLoopError::config("no base url");
        assert!(matches!(
            classify_failure(&err),
            CredentialCheckResult::Unreachable { .. }
        ));
    }

    #[test]
    fn rejection_message_carries_no_provider_body() {
        let err = AgentLoopError::llm_kind(
            LlmErrorKind::Authentication,
            "Models API returned 401: {\"error\":\"sk-secret-echo\"}",
        );
        let CredentialCheckResult::Rejected { message } = classify_failure(&err) else {
            panic!("expected rejection");
        };
        assert!(!message.contains("sk-secret-echo"));
    }
}
