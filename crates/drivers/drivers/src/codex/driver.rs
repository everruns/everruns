//! Typed OAuth assembly for the Codex and ChatGPT Plan routes.
//!
//! One builder wires [`RotatingAuth`] over a host [`TokenStore`] and maps the
//! grant's [`TokenRoute`] to its backend, so the [`Provider`] it returns serves
//! chat and `list_models` with the same credentials. There is deliberately no
//! shared auth enum across drivers: each driver exposes only the auth it
//! supports, and invalid combinations fail in [`CodexDriverBuilder::build`]
//! with a named [`CodexDriverError`].
//!
//! ```no_run
//! # fn grant() -> everruns_drivers::chatgpt::CodexAuth { unimplemented!() }
//! use everruns_drivers::codex::{self, MemoryTokenStore, TokenRoute};
//! use std::sync::Arc;
//!
//! // Store-backed: refreshes rotate through the host's store.
//! let store = Arc::new(MemoryTokenStore::new(grant()));
//! let provider = codex::driver(store)
//!     .route(TokenRoute::Codex)
//!     .originator("my-app")
//!     .build()?;
//!
//! // Ephemeral: a grant that lives only as long as this provider.
//! let provider = codex::driver::ephemeral(grant()).build()?;
//! # Ok::<(), codex::CodexDriverError>(())
//! ```
use super::{CODEX_DRIVER_ID, route_provider};
use crate::chatgpt::{
    CodexAuth,
    auth::{MemoryTokenStore, RotatingAuth, TokenRoute, TokenStore},
    oauth,
};
use everruns_contracts::{Provider, ProviderKey};
use std::sync::Arc;

/// Why a builder could not produce a provider. Raised at construction, before
/// any network call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CodexDriverError {
    /// The grant cannot authenticate on the selected route.
    #[error("{route:?} route cannot use this grant: {reason}")]
    UnsupportedAuth {
        route: TokenRoute,
        reason: &'static str,
    },
    /// The option has no effect on the selected route.
    #[error("{route:?} route does not support the `{option}` option")]
    UnsupportedOption {
        route: TokenRoute,
        option: &'static str,
    },
    /// The option's value is malformed (for example a non-HTTP URL).
    #[error("invalid `{option}` value")]
    InvalidValue { option: &'static str },
}

/// Builds an OAuth-authenticated Codex or ChatGPT Plan [`Provider`].
#[must_use = "call `build` to construct the provider"]
pub struct CodexDriverBuilder {
    store: Arc<dyn TokenStore>,
    // Present for ephemeral grants, which can be validated synchronously.
    grant: Option<CodexAuth>,
    id: Option<ProviderKey>,
    route: TokenRoute,
    originator: Option<String>,
    base_url: Option<String>,
    token_url: Option<String>,
    client_version: Option<String>,
}

/// Start a provider over a host-owned token store. The route defaults to
/// [`TokenRoute::Codex`].
pub fn driver(store: Arc<dyn TokenStore>) -> CodexDriverBuilder {
    CodexDriverBuilder {
        store,
        grant: None,
        id: None,
        route: TokenRoute::Codex,
        originator: None,
        base_url: None,
        token_url: None,
        client_version: None,
    }
}

/// Start a provider over a grant held only in memory. Refreshed tokens are
/// kept for this provider's lifetime and never persisted.
pub fn ephemeral(auth: CodexAuth) -> CodexDriverBuilder {
    let mut builder = driver(Arc::new(MemoryTokenStore::new(auth.clone())));
    builder.grant = Some(auth);
    builder
}

impl CodexDriverBuilder {
    /// Runtime provider key. Defaults to `openai-codex` or `chatgpt` by route.
    pub fn id(mut self, id: impl Into<ProviderKey>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn route(mut self, route: TokenRoute) -> Self {
        self.route = route;
        self
    }
    /// `originator` header the Codex backend attributes requests to.
    /// Codex route only.
    pub fn originator(mut self, name: impl Into<String>) -> Self {
        self.originator = Some(name.into());
        self
    }
    /// Backend base URL override (proxies, tests).
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
    /// OAuth token endpoint override used for refresh.
    pub fn token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = Some(url.into());
        self
    }
    /// Codex client version reported to the catalog. Codex route only.
    pub fn client_version(mut self, version: impl Into<String>) -> Self {
        self.client_version = Some(version.into());
        self
    }

    pub fn build(self) -> Result<Provider, CodexDriverError> {
        let route = self.route;
        if route == TokenRoute::ChatGptPlan {
            for (set, option) in [
                (self.originator.is_some(), "originator"),
                (self.client_version.is_some(), "client_version"),
            ] {
                if set {
                    return Err(CodexDriverError::UnsupportedOption { route, option });
                }
            }
        }
        for (value, option) in [(&self.base_url, "base_url"), (&self.token_url, "token_url")] {
            if value.as_deref().is_some_and(|url| !is_http_url(url)) {
                return Err(CodexDriverError::InvalidValue { option });
            }
        }
        for (value, option) in [
            (&self.originator, "originator"),
            (&self.client_version, "client_version"),
        ] {
            if value.as_deref().is_some_and(|v| !is_header_token(v)) {
                return Err(CodexDriverError::InvalidValue { option });
            }
        }
        if let Some(grant) = &self.grant {
            validate_grant(route, grant)?;
        }

        let mut auth = RotatingAuth::new(self.store, route);
        if let Some(originator) = self.originator {
            auth = auth.with_originator(originator);
        }
        if let Some(token_url) = self.token_url {
            auth = auth.with_token_url(token_url);
        }
        let id = self.id.unwrap_or_else(|| {
            ProviderKey::new(match route {
                TokenRoute::Codex => CODEX_DRIVER_ID,
                TokenRoute::ChatGptPlan => "chatgpt",
            })
        });
        Ok(route_provider(
            id,
            route,
            self.base_url.as_deref(),
            self.client_version.as_deref(),
        )
        .auth(auth))
    }
}

/// The synchronous subset of `RotatingAuth::token`'s checks, so an ephemeral
/// grant that can never authenticate fails at construction.
fn validate_grant(route: TokenRoute, grant: &CodexAuth) -> Result<(), CodexDriverError> {
    let unsupported = |reason| Err(CodexDriverError::UnsupportedAuth { route, reason });
    if grant.access_token.is_empty() {
        return unsupported("the grant has no access token");
    }
    if grant.refresh_token.is_none()
        && grant
            .expires_at
            .is_some_and(|t| t <= oauth::now_epoch_millis() + 60_000)
    {
        return unsupported("the grant has expired and has no refresh token");
    }
    if route == TokenRoute::ChatGptPlan {
        if grant
            .client_id
            .as_deref()
            .is_none_or(|id| oauth::validate_client_id(id).is_err())
        {
            return unsupported("a ChatGPT Plan grant needs its issuing client");
        }
        if !grant
            .open_source
            .as_ref()
            .is_some_and(|g| oauth::grants_plan_usage(&g.scopes))
        {
            return unsupported("the grant does not allow ChatGPT Plan use");
        }
    }
    Ok(())
}

fn is_http_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

fn is_header_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_graphic())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chatgpt::OpenSourceGrant;

    fn grant() -> CodexAuth {
        CodexAuth {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: None,
            account_id: Some("account".into()),
            email: None,
            client_id: None,
            open_source: None,
        }
    }

    #[test]
    fn plan_only_options_fail_with_named_errors() {
        let store = Arc::new(MemoryTokenStore::empty());
        let err = driver(store.clone())
            .route(TokenRoute::ChatGptPlan)
            .originator("app")
            .build()
            .unwrap_err();
        assert_eq!(
            err,
            CodexDriverError::UnsupportedOption {
                route: TokenRoute::ChatGptPlan,
                option: "originator"
            }
        );
        let err = driver(store)
            .route(TokenRoute::ChatGptPlan)
            .client_version("1.0.0")
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            CodexDriverError::UnsupportedOption {
                option: "client_version",
                ..
            }
        ));
    }

    #[test]
    fn ephemeral_grant_is_checked_against_its_route() {
        // A Codex CLI grant has no plan scope or issuing client.
        let err = ephemeral(grant())
            .route(TokenRoute::ChatGptPlan)
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            CodexDriverError::UnsupportedAuth {
                route: TokenRoute::ChatGptPlan,
                ..
            }
        ));
        assert!(ephemeral(grant()).build().is_ok());

        let mut empty = grant();
        empty.access_token.clear();
        assert!(matches!(
            ephemeral(empty).build().unwrap_err(),
            CodexDriverError::UnsupportedAuth { .. }
        ));
        let mut stale = grant();
        stale.refresh_token = None;
        stale.expires_at = Some(0);
        assert!(matches!(
            ephemeral(stale).build().unwrap_err(),
            CodexDriverError::UnsupportedAuth { .. }
        ));

        let mut plan = grant();
        plan.client_id = Some("app_test".into());
        plan.open_source = Some(OpenSourceGrant {
            id_token: None,
            scopes: vec![oauth::PLAN_SCOPE.into()],
            subject: None,
        });
        let provider = ephemeral(plan)
            .route(TokenRoute::ChatGptPlan)
            .build()
            .unwrap();
        assert_eq!(provider.id().as_str(), "chatgpt");
    }

    #[test]
    fn malformed_values_fail_before_any_request() {
        let store = Arc::new(MemoryTokenStore::empty());
        for builder in [
            driver(store.clone()).base_url("chatgpt.com/backend-api"),
            driver(store.clone()).token_url("file:///etc/passwd"),
            driver(store.clone()).originator("bad value"),
            driver(store.clone()).client_version(""),
        ] {
            assert!(matches!(
                builder.build().unwrap_err(),
                CodexDriverError::InvalidValue { .. }
            ));
        }
        let provider = driver(store)
            .id("my-codex")
            .base_url("https://proxy.example/codex")
            .build()
            .unwrap();
        assert_eq!(provider.id().as_str(), "my-codex");
        assert_eq!(
            provider.endpoint().base_url(),
            Some("https://proxy.example/codex")
        );
    }

    #[tokio::test]
    async fn memory_store_compare_and_save_rejects_stale_previous() {
        let store = MemoryTokenStore::new(grant());
        let mut next = grant();
        next.access_token = "next".into();
        let mut stale = grant();
        stale.access_token = "stale".into();
        assert!(!store.compare_and_save(&stale, next.clone()).await.unwrap());
        assert!(
            store
                .compare_and_save(&grant(), next.clone())
                .await
                .unwrap()
        );
        assert_eq!(store.load().await.unwrap(), Some(next.clone()));
        store.clear().await;
        assert!(store.load().await.unwrap().is_none());
        assert!(!store.compare_and_save(&next, grant()).await.unwrap());
        // The lease is exclusive until its guard drops.
        let lease = store.lock().await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), store.lock())
                .await
                .is_err()
        );
        drop(lease);
        drop(store.lock().await.unwrap());
    }

    #[tokio::test]
    async fn list_models_sends_oauth_bearer() {
        use everruns_contracts::ChatDriver;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer test-access-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"slug": "gpt-6.1-sol", "display_name": "Sol", "visibility": "list"},
                    {"slug": "hidden-model", "visibility": "unlisted"}
                ]
            })))
            .mount(&server)
            .await;

        // A far-future grant never refreshes, so no traffic leaves the mock.
        let grant = CodexAuth {
            access_token: "test-access-token".into(),
            refresh_token: None,
            expires_at: Some(4_000_000_000_000),
            account_id: None,
            email: None,
            client_id: None,
            open_source: None,
        };
        let provider = driver(Arc::new(MemoryTokenStore::new(grant)))
            .base_url(server.uri())
            .build()
            .expect("provider builds");
        let models = crate::codex::CodexChatDriver::new()
            .list_models(provider.endpoint())
            .await
            .expect("listing works")
            .expect("codex supports listing");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "gpt-6.1-sol");
    }
}
