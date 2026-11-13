//! Explicit deployment-owned extraction utility. Tenant APIs must use their
//! organization provider bindings, never fall back to these credentials.
use crate::Extractions;
use everruns_contracts::error::{AgentLoopError, Result};

/// Select the deployment-owned OpenRouter extraction service. No default model
/// or key is silently borrowed from tenant chat configuration.
pub fn utility_extractions_from_env() -> Result<Option<Extractions>> {
    from_config(
        std::env::var("UTILITY_EXTRACTION_MODEL").ok(),
        std::env::var("UTILITY_OPENROUTER_API_KEY").ok(),
    )
}

fn from_config(model: Option<String>, key: Option<String>) -> Result<Option<Extractions>> {
    let model = model.filter(|s| !s.trim().is_empty());
    let key = key.filter(|s| !s.trim().is_empty());
    match (model, key) {
        (None, _) => Ok(None),
        (Some(model), Some(key)) => Ok(Some(Extractions::new(
            model,
            everruns_drivers::openrouter::provider("utility-extractions", key),
        ))),
        _ => Err(AgentLoopError::Configuration(
            "Utility extractions require both UTILITY_OPENROUTER_API_KEY and UTILITY_EXTRACTION_MODEL"
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_model_means_disabled_even_if_other_utility_key_exists() -> Result<()> {
        assert!(from_config(None, Some("unused".into()))?.is_none());
        assert!(from_config(Some(" ".into()), None)?.is_none());
        Ok(())
    }

    #[test]
    fn model_requires_explicit_nonempty_utility_key() {
        assert!(from_config(Some("cloudflare/clairic".into()), None).is_err());
        assert!(from_config(Some("cloudflare/clairic".into()), Some(" ".into())).is_err());
    }

    #[test]
    fn explicit_configuration_constructs_service_without_provider_call() -> Result<()> {
        let service = from_config(Some("cloudflare/clairic".into()), Some("test-key".into()))?;
        assert!(service.is_some_and(|service| service.is_configured()));
        Ok(())
    }
}
