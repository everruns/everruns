//! An org's model providers on its data plane (step 8): each active
//! provider is an `llm_provider` connection, and its API key a secret
//! sealed with the org's data key, so only the executor running a turn
//! opens it.
//!
//! Decisions:
//! - Synced whole, from the providers table, whenever an org's providers
//!   change and when the org is put on a data plane. A provider the org no
//!   longer has (or that is not active) is removed; running turns keep what
//!   they started with.
//! - The connection id is the provider's key (its normalized name), the
//!   key an agent's `model.provider` names, so a published agent finds its
//!   provider without a platform id.
//! - A provider with a personal sign-in (`chatgpt`) is not an org's to
//!   share and stays on the platform's workers.
//! - The key is sealed again only when it changed (the control plane holds
//!   the org's data key, so it opens the current one to compare); an
//!   unchanged provider writes nothing.

use std::collections::HashSet;

use everruns_contracts::dataplane::{self as dp, LLM_PROVIDER, LlmProviderSettings};
use everruns_contracts::runtime_provider::ProviderKey;

use super::{ControlPlane, put_connection, put_secret, remove};
use crate::services::provider_resolver::{
    provider_api, provider_request_options, resolve_provider_api_key,
};

/// Bring `org_id`'s provider connections in line with its providers. A v1
/// org is left alone.
pub async fn sync(cp: &ControlPlane, org_id: i64) -> anyhow::Result<()> {
    if cp.db.dp_org(org_id).await?.is_none() {
        return Ok(());
    }
    let mut live = HashSet::new();
    for provider in cp.db.list_providers(org_id).await? {
        if !provider.status.eq_ignore_ascii_case("active") || provider.provider_type == "chatgpt" {
            continue;
        }
        let key = ProviderKey::new(&provider.name);
        let id = dp::llm_provider_connection(key.as_str());
        let api_key = resolve_provider_api_key(&cp.db, cp.encryption.as_deref(), &provider)?
            .unwrap_or_default();
        let settings = LlmProviderSettings {
            provider_key: key.as_str().to_owned(),
            provider_type: provider.provider_type.clone(),
            base_url: provider.base_url.clone(),
            api: provider_api(&provider.settings)?,
            request_options: provider_request_options(&provider.settings),
        };
        let connection = dp::Connection {
            connection_id: id.clone(),
            kind: LLM_PROVIDER.to_owned(),
            settings: serde_json::to_value(&settings)?,
            token_secret_id: id.clone(),
            refresh_owner: dp::RefreshOwner::ControlPlane,
        };
        if sealed_key(cp, org_id, &id).await?.as_deref() != Some(api_key.as_bytes()) {
            put_secret(cp, org_id, &id, api_key.as_bytes()).await?;
        }
        let current = cp.db.dp_item(org_id, "connection", &id).await?;
        if current.and_then(|item| item.body)
            != Some(dp::ConfigChange::Connection(connection.clone()).to_value())
        {
            put_connection(&cp.db, org_id, connection).await?;
        }
        live.insert(id);
    }
    for item in cp.db.dp_items_live(org_id).await? {
        let gone = item.item_id.starts_with("llm:") && !live.contains(&item.item_id);
        if gone && (item.kind == "connection" || item.kind == "secret") {
            remove(&cp.db, org_id, &item.kind, &item.item_id).await?;
        }
    }
    Ok(())
}

/// [`sync`], logged rather than failed: a provider change in the console
/// never fails because the data plane's copy did not update; the next
/// change, or putting the org on a data plane again, syncs it.
pub async fn sync_logged(cp: &ControlPlane, org_id: i64) {
    if let Err(error) = sync(cp, org_id).await {
        tracing::warn!(org_id, %error, "the data plane's providers were not synced");
    }
}

/// The key sealed as `org_id`'s secret `id` now, opened, if there is one.
async fn sealed_key(cp: &ControlPlane, org_id: i64, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let Some(body) = cp
        .db
        .dp_item(org_id, "secret", id)
        .await?
        .and_then(|item| item.body)
    else {
        return Ok(None);
    };
    let Some(dp::ConfigChange::Secret(sealed)) = dp::ConfigChange::parse(&body) else {
        return Ok(None);
    };
    let Some(org) = cp.db.dp_org(org_id).await? else {
        return Ok(None);
    };
    let key = cp.data_key(&org)?;
    // A key sealed under an older data key reads as changed.
    Ok(everruns_contracts::dataplane_seal::unseal(&key, &super::org_public(org_id), &sealed).ok())
}
