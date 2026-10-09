//! Default OSS platform definition helpers.
//!
//! The default OSS platform stays centralized here so server startup, org
//! initialization, and docs can all point to the same preset. Hosted integration
//! composition lives in `everruns-capabilities`; embedders can start from
//! the OSS preset, filter `everruns_capabilities::integrations_catalog::CATALOG`, or
//! construct a `HostComposition` manually.

use crate::records::BuiltInHarnessDefinition;
use crate::records::email::{EmailSender, SystemEmailConfig};
use everruns::utility_llm::SystemUtilityLlmConfig;
use everruns_contracts::connector::ConnectorRegistry;
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::deployment::DeploymentGrade;
use everruns_core::host::DirectEgressService;
use everruns_core::host::HostComposition;
use std::sync::Arc;
use uuid::Uuid;

/// The platform fallback when an organization has not selected a model:
/// GPT-6 Luna (`seed::seed_ids::GPT_6_LUNA`).
pub(crate) const PLATFORM_DEFAULT_MODEL_ID: Uuid =
    Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000022c);

pub(crate) const fn platform_default_model_id(org_id: i64) -> Option<Uuid> {
    if org_id == DEFAULT_ORG_ID {
        Some(PLATFORM_DEFAULT_MODEL_ID)
    } else {
        None
    }
}

/// Build the default OSS `HostComposition` for the current deployment grade.
pub fn oss_host_composition() -> HostComposition {
    oss_host_composition_for_grade(DeploymentGrade::from_env())
}

/// Build the default OSS `HostComposition` for an explicit deployment grade.
pub fn oss_host_composition_for_grade(grade: DeploymentGrade) -> HostComposition {
    let capability_registry = oss_capability_registry_for_grade(grade);
    let driver_registry = everruns_worker::create_driver_registry();
    // Runtime egress honors EVERRUNS_SYSTEM_ALLOWLIST_ENABLED for tenant/agent
    // paths.
    let egress_service = Arc::new(DirectEgressService::for_runtime_traffic_from_env());
    let utility_llm_service = SystemUtilityLlmConfig::from_env().into_service();
    // Deployment-owned typed decisions, routed across the configured decision
    // drivers (`UTILITY_DECISION_DRIVER`, `UTILITY_TYPESAFE_API_KEY`, the utility LLM).
    // Nothing configured = disabled service; guardrail checks configured for
    // it then fail open, the same contract as a missing utility model. A
    // driver that is chosen but not configured stops startup here.
    let decisions = everruns_worker::SystemDecisions::from_env()
        .into_service(utility_llm_service.clone())
        .unwrap_or_else(|error| panic!("invalid decisions configuration: {error}"));

    // EVE-879: the connector registry and system email sender are hosted
    // control-plane services, composed on `ServerAppBuilder` (see
    // `oss_connector_registry` / `system_email_sender`), not carried on the
    // execution-facing `HostComposition`.
    let mut builder = HostComposition::builder()
        .capability_registry(capability_registry)
        .driver_registry(driver_registry)
        .egress_service(egress_service)
        .utility_llm_service(utility_llm_service)
        .decisions(decisions)
        .session_file_system_factory(Arc::new(
            crate::domains::session_files::StorageSessionFileSystemFactory,
        ));

    // Knowledge Index vector store. Opt-in: when `TURBOPUFFER_API_KEY` is set
    // (and non-empty) use the Turbopuffer backend, otherwise keep the in-memory
    // default. The API key is never logged.
    let vector_store: Arc<dyn everruns_capabilities::VectorStore> =
        if let Some(store) = turbopuffer_vector_store_from_env() {
            store
        } else {
            tracing::info!(vector_store = "in-memory", "vector store backend active");
            Arc::new(everruns_capabilities::InMemoryVectorStore::new())
        };
    builder = builder.extension(Arc::new(everruns_capabilities::VectorStoreExt(
        vector_store,
    )));

    builder.build()
}

/// Build a Turbopuffer-backed vector store from the environment, or `None` when
/// `TURBOPUFFER_API_KEY` is unset/empty (keeps the in-memory default).
///
/// `TURBOPUFFER_BASE_URL` selects the regional endpoint; it defaults to a
/// sensible region. The API key is read but never logged.
fn turbopuffer_vector_store_from_env() -> Option<Arc<everruns_turbopuffer::TurbopufferVectorStore>>
{
    /// Default Turbopuffer region used when `TURBOPUFFER_BASE_URL` is unset.
    const DEFAULT_BASE_URL: &str = "https://gcp-us-central1.turbopuffer.com";

    let api_key = std::env::var("TURBOPUFFER_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())?;
    let base_url = std::env::var("TURBOPUFFER_BASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    tracing::info!(vector_store = "turbopuffer", %base_url, "vector store backend active");
    Some(Arc::new(everruns_turbopuffer::TurbopufferVectorStore::new(
        base_url, api_key,
    )))
}

/// Build the default OSS connector registry.
pub fn oss_connector_registry() -> ConnectorRegistry {
    oss_connector_registry_for_grade(DeploymentGrade::from_env())
}

/// Build the default OSS connector registry for an explicit grade.
pub fn oss_connector_registry_for_grade(grade: DeploymentGrade) -> ConnectorRegistry {
    let mut registry = ConnectorRegistry::new();
    everruns_capabilities::integrations_catalog::register_connectors(&mut registry, grade);
    registry
}

/// Build the default OSS capability registry for the current deployment grade.
pub fn oss_capability_registry() -> everruns_core::capabilities::CapabilityRegistry {
    everruns_capabilities::integrations_catalog::oss_capability_registry()
}

/// Build the default OSS capability registry for an explicit grade.
///
/// Portable builtins, the integrations named in
/// `everruns_capabilities::integrations_catalog::CATALOG`, then the hosted product catalog.
pub fn oss_capability_registry_for_grade(
    grade: DeploymentGrade,
) -> everruns_core::capabilities::CapabilityRegistry {
    everruns_capabilities::integrations_catalog::oss_capability_registry_for_grade(grade)
}

/// Built-in harness templates for the default OSS platform.
pub fn oss_built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    crate::harnesses::built_in_harnesses()
}

/// Build the environment-configured system email sender (EVE-879).
///
/// System email uses its direct provider client outside the runtime egress
/// boundary — it is operator-owned deployment traffic, not tenant traffic.
/// Returns the `DisabledEmailSender` when `EMAIL_PROVIDER` is unset.
pub fn system_email_sender() -> Arc<dyn EmailSender> {
    SystemEmailConfig::from_env()
        .expect("Invalid system email configuration")
        .into_sender()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wiring a full-stack run would exercise: the OSS composition a server
    /// actually builds carries a decisions, and it is the disabled one
    /// unless the deployment configured a key. A composition that silently
    /// carried no service would make every `jev` guardrail check a no-op.
    #[test]
    fn oss_composition_carries_a_classifier() {
        let composition = oss_host_composition_for_grade(DeploymentGrade::Dev);
        let service = composition.decisions();
        // Process env decides which one; both are valid, a missing service is not.
        let configured =
            std::env::var(everruns_integrations::typesafe::UTILITY_TYPESAFE_API_KEY_ENV)
                .is_ok_and(|key| !key.trim().is_empty());
        assert_eq!(
            service.is_configured(),
            configured,
            "decisions configuration must follow {}",
            everruns_integrations::typesafe::UTILITY_TYPESAFE_API_KEY_ENV
        );
        assert_eq!(
            service.name(),
            if configured {
                "DecisionRouter"
            } else {
                "DisabledDecisionsService"
            }
        );
    }

    /// The `jev` capability reaches the hosted registry in both dev and prod
    /// deployments, which is what its prod-grade `typesafe` flag promises.
    #[test]
    fn jev_capability_is_registered_for_dev_and_prod_deployments() {
        assert!(
            oss_host_composition_for_grade(DeploymentGrade::Dev)
                .capability_registry()
                .has("jev"),
            "dev deployments should offer the jev capability"
        );
        assert!(
            oss_host_composition_for_grade(DeploymentGrade::Prod)
                .capability_registry()
                .has("jev"),
            "prod deployments should offer the jev capability"
        );
    }

    /// The connector an operator sees in Settings > My agent experience is registered
    /// too, otherwise the capability has no way to get a user's key.
    #[test]
    fn typesafe_connector_is_registered_for_dev_deployments() {
        let registry = oss_connector_registry_for_grade(DeploymentGrade::Dev);
        assert!(
            registry.get("typesafe").is_some(),
            "the capability resolves its key from this connection provider"
        );
    }
}
