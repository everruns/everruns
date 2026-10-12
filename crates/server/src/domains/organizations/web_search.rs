// Organization web search: search accounts and `web_search_defaults`.
//
// Decision: search accounts are organization connections (the same rows and
// endpoints as sandbox provider accounts) whose provider has a search driver
// (`WEB_SEARCH_ACCOUNT_PROVIDERS`, whose connectors declare the `web_search`
// connector capability). Nothing else is stored for them.
//
// Decision: the running tool reads these through two internal worker commands
// (settings without keys, then one account's key when its plan step runs), so
// plans resolve live from the current defaults; the worker caches the
// settings for `ORG_SETTINGS_TTL` (30 s). See knowledge/integrations/web-search.md.

use crate::storage::StorageBackend;
use everruns_contracts::runtime::web_search::{
    SearchAccount, WEB_SEARCH_ACCOUNT_PROVIDERS, WebSearchDefaults, WebSearchOrgSettings,
};

/// The organization's search accounts, in listing order (name, then age).
pub async fn search_accounts(
    db: &StorageBackend,
    org_id: i64,
) -> anyhow::Result<Vec<SearchAccount>> {
    Ok(db
        .list_organization_connections(org_id)
        .await?
        .into_iter()
        .filter(|row| WEB_SEARCH_ACCOUNT_PROVIDERS.contains(&row.provider.as_str()))
        .map(|row| SearchAccount {
            id: row.id,
            name: row.name.unwrap_or_else(|| row.provider.clone()),
            provider: row.provider,
        })
        .collect())
}

/// Defaults and accounts, as the running tool plans with them.
pub async fn web_search_settings(
    db: &StorageBackend,
    org_id: i64,
) -> anyhow::Result<WebSearchOrgSettings> {
    Ok(WebSearchOrgSettings {
        defaults: db.org_web_search_defaults(org_id).await?,
        accounts: search_accounts(db, org_id).await?,
    })
}

/// Validate defaults against the organization's current search accounts.
/// `Err(Ok(message))` is a bad request; `Err(Err(_))` a storage failure.
pub async fn validate_defaults(
    db: &StorageBackend,
    org_id: i64,
    defaults: WebSearchDefaults,
) -> Result<WebSearchDefaults, Result<String, anyhow::Error>> {
    let accounts: Vec<uuid::Uuid> = search_accounts(db, org_id)
        .await
        .map_err(Err)?
        .into_iter()
        .map(|account| account.id)
        .collect();
    defaults.validated(&accounts).map_err(Ok)
}
