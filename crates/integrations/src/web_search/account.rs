//! Organization search accounts and the organization settings cache.
//!
//! Decision: an account step fetches its key only when the plan reaches it,
//! through the session's connection resolver, so a plan that free search or
//! an earlier account answers never decrypts a key it does not use. Keys are
//! never cached; the organization's defaults and account list (no keys) are,
//! for [`ORG_SETTINGS_TTL`] per organization, so a changed default reaches
//! running sessions within that window without one lookup per search.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::connection_services::UserConnectionResolver;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::web_search::{
    SearchAccount, SearchContext, SearchDriver, SearchDriverCapabilities, SearchFailure,
    SearchFailureKind, SearchRequest, SearchResponse, WEB_SEARCH_SETTINGS_TTL,
    WebSearchOrgSettings, cached_web_search_settings,
};
use everruns_contracts::typed_id::SessionId;
use tracing::debug;

use super::brave::{BRAVE_API_BASE, BraveSearchDriver};
use super::cloudflare::{CLOUDFLARE_API_BASE, CloudflareSearchDriver, CloudflareSearchOptions};
use super::parallel::{PARALLEL_API_BASE, ParallelSearchDriver};
use super::{
    BRAVE_SEARCH_CONNECTION_PROVIDER, CLOUDFLARE_WEB_SEARCH_CONNECTION_PROVIDER,
    PARALLEL_CONNECTION_PROVIDER,
};

/// How long a worker reuses an organization's web search settings.
pub const ORG_SETTINGS_TTL: Duration = WEB_SEARCH_SETTINGS_TTL;

/// The organization's web search settings for this call, or `None` when the
/// host has none (Framework, serve) or the lookup failed (logged; the call
/// then uses the built-in defaults rather than failing).
pub(super) async fn org_settings(context: &ToolContext) -> Option<Arc<WebSearchOrgSettings>> {
    let resolver = context.connection_resolver.as_ref()?;
    let key = match &context.org_id {
        Some(org) => format!("org:{org:?}"),
        None => format!("session:{}", context.session_id),
    };
    cached_web_search_settings(resolver.as_ref(), &key, context.session_id).await
}

/// API endpoints of the keyed drivers; tests point them at mock servers.
#[derive(Clone)]
pub(super) struct DriverBases {
    pub(super) brave: String,
    pub(super) parallel: String,
    pub(super) cloudflare: String,
}

impl Default for DriverBases {
    fn default() -> Self {
        Self {
            brave: BRAVE_API_BASE.to_string(),
            parallel: PARALLEL_API_BASE.to_string(),
            cloudflare: CLOUDFLARE_API_BASE.to_string(),
        }
    }
}

impl DriverBases {
    /// The driver for a search account's connection provider, key and
    /// stored options, or `None` when no driver exists for that provider or
    /// its options are missing or invalid.
    pub(super) fn driver(
        &self,
        provider: &str,
        key: String,
        options: Option<&serde_json::Value>,
    ) -> Option<Arc<dyn SearchDriver>> {
        match provider {
            BRAVE_SEARCH_CONNECTION_PROVIDER => {
                Some(Arc::new(BraveSearchDriver::with_base_url(key, &self.brave)))
            }
            PARALLEL_CONNECTION_PROVIDER => Some(Arc::new(ParallelSearchDriver::with_base_url(
                key,
                &self.parallel,
            ))),
            CLOUDFLARE_WEB_SEARCH_CONNECTION_PROVIDER => {
                let options = match CloudflareSearchOptions::from_value(options?) {
                    Ok(options) => options,
                    Err(error) => {
                        debug!("cloudflare search account options rejected: {error}");
                        return None;
                    }
                };
                Some(Arc::new(CloudflareSearchDriver::with_base_url(
                    key,
                    options,
                    &self.cloudflare,
                )))
            }
            _ => None,
        }
    }
}

/// The search driver for an organization search account's connection
/// provider (`brave_search`, `parallel`, `cloudflare_web_search`), key and
/// stored options, or `None` when this build has no driver for the provider.
/// The server's account test uses it.
pub fn account_search_driver(
    provider: &str,
    key: String,
    options: Option<&serde_json::Value>,
) -> Option<Arc<dyn SearchDriver>> {
    DriverBases::default().driver(provider, key, options)
}

/// A search driver for one organization account, or `None` when this host
/// has no driver for the account's provider.
pub(super) fn account_driver(
    account: &SearchAccount,
    resolver: Arc<dyn UserConnectionResolver>,
    session_id: SessionId,
    bases: &DriverBases,
) -> Option<Arc<dyn SearchDriver>> {
    // A probe driver supplies the label, capabilities and hosts; the real
    // one is built with the key when the plan reaches this account.
    let probe = bases.driver(&account.provider, String::new(), account.options.as_ref())?;
    Some(Arc::new(AccountDriver {
        id: format!("account:{}", account.id),
        label: format!("{} · {}", probe.label(), account.name),
        provider: account.provider.clone(),
        options: account.options.clone(),
        probe,
        account: account.id,
        session_id,
        resolver,
        bases: bases.clone(),
    }))
}

/// A search account's driver with the key fetched on use.
struct AccountDriver {
    id: String,
    label: String,
    provider: String,
    options: Option<serde_json::Value>,
    probe: Arc<dyn SearchDriver>,
    account: uuid::Uuid,
    session_id: SessionId,
    resolver: Arc<dyn UserConnectionResolver>,
    bases: DriverBases,
}

#[async_trait]
impl SearchDriver for AccountDriver {
    fn id(&self) -> &str {
        &self.id
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn capabilities(&self) -> SearchDriverCapabilities {
        self.probe.capabilities()
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        self.probe.egress_hosts()
    }

    async fn search(
        &self,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure> {
        let key = match self
            .resolver
            .get_web_search_account_key(self.session_id, self.account)
            .await
        {
            Ok(Some(key)) if !key.is_empty() => key,
            Ok(_) => {
                return Err(SearchFailure::new(
                    SearchFailureKind::NotConfigured,
                    "The search account is not connected in this organization",
                ));
            }
            Err(error) => {
                debug!("search account key lookup failed: {error}");
                return Err(SearchFailure::new(
                    SearchFailureKind::Unavailable,
                    "The search account key could not be read",
                ));
            }
        };
        match self
            .bases
            .driver(&self.provider, key, self.options.as_ref())
        {
            Some(driver) => driver.search(request, context).await,
            None => Err(SearchFailure::new(
                SearchFailureKind::NotConfigured,
                "This host has no driver for the search account",
            )),
        }
    }
}
