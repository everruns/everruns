//! Default OSS platform definition helpers.
//!
//! The default OSS platform stays centralized here so server startup, org
//! initialization, and docs can all point to the same preset. Inventory-based
//! integration discovery is intentionally confined to this module; embedders
//! can start from the OSS preset or construct a `PlatformDefinition` manually
//! without depending on inventory registration.

use everruns_core::deployment::DeploymentGrade;
use everruns_core::{
    DEFAULT_ORG_ID, DirectEgressService, PlatformDefinition, SystemUtilityLlmConfig,
};
use everruns_platform::BuiltInHarnessDefinition;
use everruns_platform::connector::{ConnectorPlugin, ConnectorRegistry};
use everruns_platform::email::{EmailSender, SystemEmailConfig};
use std::sync::Arc;
use uuid::Uuid;

/// The platform fallback when an organization has not selected a model.
pub(crate) const PLATFORM_DEFAULT_MODEL_ID: Uuid =
    Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000227);

pub(crate) const fn platform_default_model_id(org_id: i64) -> Option<Uuid> {
    if org_id == DEFAULT_ORG_ID {
        Some(PLATFORM_DEFAULT_MODEL_ID)
    } else {
        None
    }
}

/// Build the default OSS `PlatformDefinition` for the current deployment grade.
pub fn oss_platform_definition() -> PlatformDefinition {
    oss_platform_definition_for_grade(DeploymentGrade::from_env())
}

/// Build the default OSS `PlatformDefinition` for an explicit deployment grade.
pub fn oss_platform_definition_for_grade(grade: DeploymentGrade) -> PlatformDefinition {
    let capability_registry =
        everruns_platform::capabilities::hosted_capability_registry_for_grade(grade);
    let driver_registry = everruns_worker::create_driver_registry();
    // Runtime egress honors EVERRUNS_SYSTEM_ALLOWLIST_ENABLED for tenant/agent
    // paths.
    let egress_service = Arc::new(DirectEgressService::for_runtime_traffic_from_env());
    let utility_llm_service = SystemUtilityLlmConfig::from_env().into_service();

    // EVE-879: the connector registry and system email sender are hosted
    // control-plane services, composed on `ServerAppBuilder` (see
    // `oss_connector_registry` / `system_email_sender`), not carried on the
    // execution-facing `PlatformDefinition`.
    let mut builder = PlatformDefinition::builder()
        .capability_registry(capability_registry)
        .driver_registry(driver_registry)
        .egress_service(egress_service)
        .utility_llm_service(utility_llm_service)
        .session_file_system_factory(Arc::new(
            crate::domains::session_files::StorageSessionFileSystemFactory,
        ));

    // Knowledge Index vector store. Opt-in: when `TURBOPUFFER_API_KEY` is set
    // (and non-empty) use the Turbopuffer backend, otherwise keep the in-memory
    // default. The API key is never logged.
    if let Some(store) = turbopuffer_vector_store_from_env() {
        builder = builder.vector_store(store);
    } else {
        tracing::info!(vector_store = "in-memory", "vector store backend active");
    }

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

    for plugin in inventory::iter::<ConnectorPlugin> {
        if plugin.experimental_only && !grade.experimental_features_enabled() {
            continue;
        }
        registry.register_boxed((plugin.factory)());
    }

    registry
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
