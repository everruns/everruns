//! Startup provisioning for the default org's connected providers.

use super::*;

/// Make providers connected from env usable right away, the same way a key
/// set through the API is: seed their catalog, discover their models, and
/// enable a starter set when the org has no chat model yet.
///
/// Without this an env-connected provider showed "Last synced: Never" and no
/// discovered models until the background sync first ran. Best-effort: a
/// provider that rejects the key or is unreachable must not fail startup.
pub(super) async fn provision_connected_providers(
    db: &Arc<StorageBackend>,
    encryption: &Arc<EncryptionService>,
    host_composition: &HostComposition,
    connected: &[Uuid],
) {
    if let Err(error) = seed_models_with_host_composition(db, host_composition).await {
        tracing::warn!(%error, "Seeding models for env-connected providers failed (non-fatal)");
    }
    let sync = crate::domains::models::ModelSyncService::new(
        db.clone(),
        Arc::new(host_composition.driver_registry().clone()),
        Some(encryption.clone()),
    );
    let models = crate::domains::models::ModelService::new(db.clone());
    for provider_id in connected {
        match tokio::time::timeout(
            ENV_PROVIDER_SYNC_TIMEOUT,
            sync.sync_provider(DEFAULT_ORG_ID, *provider_id),
        )
        .await
        {
            Ok(Ok(result)) => tracing::info!(
                %provider_id,
                ?result,
                "Synced models for env-connected provider"
            ),
            Ok(Err(error)) => tracing::warn!(
                %provider_id,
                %error,
                "Model sync for env-connected provider failed (non-fatal)"
            ),
            Err(_) => tracing::warn!(
                %provider_id,
                "Model sync for env-connected provider timed out (non-fatal)"
            ),
        }
        if let Err(error) = models
            .bootstrap_intelligence(DEFAULT_ORG_ID, *provider_id)
            .await
        {
            tracing::warn!(
                %provider_id,
                %error,
                "Intelligence bootstrap for env-connected provider failed (non-fatal)"
            );
        }
    }
}

/// Point the default org at a model it can actually run.
///
/// The default model falls back to the platform default (an OpenAI model)
/// when the org never chose one. Older versions seeded and enabled that model
/// without a key, so an org that connected only OpenRouter kept defaulting to
/// OpenAI and chat failed with "provider is not set up". Seeding now disables
/// keyless catalog models, which makes such a default unresolvable; this then
/// bootstraps from the first connected provider, exactly as connecting it
/// through the API would have.
pub(super) async fn ensure_default_org_default_model(
    db: &Arc<StorageBackend>,
    encryption: Option<&Arc<EncryptionService>>,
    host_composition: &HostComposition,
) {
    let resolved = match db.get_default_model(DEFAULT_ORG_ID).await {
        Ok(model) => model.is_some(),
        Err(error) => {
            tracing::warn!(%error, "Default model check failed (non-fatal)");
            return;
        }
    };
    if resolved {
        return;
    }
    let providers = match db.list_providers(DEFAULT_ORG_ID).await {
        Ok(providers) => providers,
        Err(error) => {
            tracing::warn!(%error, "Default model repair: listing providers failed (non-fatal)");
            return;
        }
    };
    let models = crate::domains::models::ModelService::new(db.clone());
    let sync = encryption.map(|encryption| {
        crate::domains::models::ModelSyncService::new(
            db.clone(),
            Arc::new(host_composition.driver_registry().clone()),
            Some(encryption.clone()),
        )
    });
    // Oldest connection first: the provider the org has relied on longest.
    for provider in providers.iter().rev().filter(|p| p.api_key_set) {
        // A provider that never synced may know no chat model yet; discover
        // its catalog first so there is something to pick.
        if provider.last_synced_at.is_none()
            && let Some(sync) = sync.as_ref()
        {
            match tokio::time::timeout(
                ENV_PROVIDER_SYNC_TIMEOUT,
                sync.sync_provider(DEFAULT_ORG_ID, provider.id.uuid()),
            )
            .await
            {
                Ok(Ok(result)) => tracing::info!(
                    provider_id = %provider.id,
                    ?result,
                    "Synced never-synced provider before picking a default model"
                ),
                Ok(Err(error)) => tracing::warn!(
                    provider_id = %provider.id,
                    %error,
                    "Model sync before default model repair failed (non-fatal)"
                ),
                Err(_) => tracing::warn!(
                    provider_id = %provider.id,
                    "Model sync before default model repair timed out (non-fatal)"
                ),
            }
        }
        match models
            .bootstrap_intelligence(DEFAULT_ORG_ID, provider.id.uuid())
            .await
        {
            Ok(outcome) if outcome.default_model_id.is_some() => {
                tracing::info!(
                    provider_id = %provider.id,
                    "Default org default model set from a connected provider"
                );
                return;
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(
                provider_id = %provider.id,
                %error,
                "Default model repair: bootstrap failed (non-fatal)"
            ),
        }
    }
}

/// Upper bound on one env-connected provider's startup model discovery.
const ENV_PROVIDER_SYNC_TIMEOUT: Duration = Duration::from_secs(30);
