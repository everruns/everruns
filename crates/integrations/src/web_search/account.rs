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

use super::BRAVE_SEARCH_CONNECTION_PROVIDER;
use super::brave::BraveSearchDriver;

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

/// A search driver for one organization account, or `None` when this host
/// has no driver for the account's provider.
pub(super) fn account_driver(
    account: &SearchAccount,
    resolver: Arc<dyn UserConnectionResolver>,
    session_id: SessionId,
    brave_base: &str,
) -> Option<Arc<dyn SearchDriver>> {
    (account.provider == BRAVE_SEARCH_CONNECTION_PROVIDER).then(|| {
        Arc::new(AccountDriver {
            id: format!("account:{}", account.id),
            label: format!("Brave Search · {}", account.name),
            account: account.id,
            session_id,
            resolver,
            brave_base: brave_base.to_string(),
        }) as Arc<dyn SearchDriver>
    })
}

/// Brave Search with an organization account's key, fetched on use.
struct AccountDriver {
    id: String,
    label: String,
    account: uuid::Uuid,
    session_id: SessionId,
    resolver: Arc<dyn UserConnectionResolver>,
    brave_base: String,
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
        BraveSearchDriver::new("").capabilities()
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        BraveSearchDriver::new("").egress_hosts()
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
        BraveSearchDriver::with_base_url(key, &self.brave_base)
            .search(request, context)
            .await
    }
}
