//! Explicit application credentials for native integrations. Stability: alpha.
use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
use everruns_core::connection_services::UserConnectionResolver;
use std::{collections::BTreeMap, fmt, sync::Arc};

/// Resolve a service credential when an operation executes. Implementations may rotate keys.
#[async_trait]
pub trait Credentials: Send + Sync {
    /// Return the token without a scheme prefix. Errors never enter tool results verbatim.
    async fn token(&self) -> Result<String, String>;
}
/// A static application credential, held outside agent configuration and events.
#[derive(Clone)]
pub struct ApiKey(Arc<str>);
impl ApiKey {
    /// Hold a secret key. Debug output is redacted.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into().into())
    }
}
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}
#[async_trait]
impl Credentials for ApiKey {
    async fn token(&self) -> Result<String, String> {
        if self.0.trim().is_empty() {
            Err("Credential is empty".into())
        } else {
            Ok(self.0.to_string())
        }
    }
}
#[derive(Clone, Default)]
pub(crate) struct Connections(pub BTreeMap<&'static str, Arc<dyn Credentials>>);
struct Resolver {
    bound: Connections,
    fallback: Option<Arc<dyn UserConnectionResolver>>,
}
impl Connections {
    pub fn resolver(
        &self,
        fallback: Option<Arc<dyn UserConnectionResolver>>,
    ) -> Arc<dyn UserConnectionResolver> {
        Arc::new(Resolver {
            bound: self.clone(),
            fallback,
        })
    }
}
#[async_trait]
impl UserConnectionResolver for Resolver {
    async fn get_connection_token(
        &self,
        session: SessionId,
        provider: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        if let Some(credentials) = self.bound.0.get(provider) {
            let token = credentials.token().await.map_err(|_| {
                everruns_contracts::error::AgentLoopError::tool(
                    "Application service credentials unavailable",
                )
            })?;
            if token.trim().is_empty() {
                return Err(everruns_contracts::error::AgentLoopError::tool(
                    "Application service credential is empty",
                ));
            }
            return Ok(Some(token));
        }
        match &self.fallback {
            Some(resolver) => resolver.get_connection_token(session, provider).await,
            None => Ok(None),
        }
    }
}
impl crate::AgentBuilder {
    #[cfg(feature = "cursor")]
    /// Enable the complete Cursor Cloud Agent tools with explicitly selected credentials.
    pub fn cursor(mut self, credentials: impl Credentials + 'static) -> Self {
        self.native_services
            .connections
            .0
            .insert("cursor", Arc::new(credentials));
        self.capability("cursor")
    }
    #[cfg(feature = "browserless")]
    /// Enable Browserless tools, including persistent browser sessions and cleanup.
    pub fn browserless(mut self, credentials: impl Credentials + 'static) -> Self {
        self.native_services
            .connections
            .0
            .insert("browserless", Arc::new(credentials));
        self.capability("browserless")
    }
    #[cfg(feature = "e2b")]
    /// Enable E2B sandbox lifecycle, filesystem and command tools with application-owned credentials.
    pub fn e2b(mut self, credentials: impl Credentials + 'static) -> Self {
        self.native_services
            .connections
            .0
            .insert("e2b", Arc::new(credentials));
        self.capability("e2b")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Rotating(Arc<AtomicUsize>);
    #[async_trait]
    impl Credentials for Rotating {
        async fn token(&self) -> Result<String, String> {
            Ok(format!("key-{}", self.0.fetch_add(1, Ordering::SeqCst)))
        }
    }
    struct Failing;
    #[async_trait]
    impl Credentials for Failing {
        async fn token(&self) -> Result<String, String> {
            Err("vault-secret-sentinel".into())
        }
    }
    #[tokio::test]
    async fn explicit_credentials_are_rotated_redacted_and_do_not_fall_back() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut bound = Connections::default();
        bound.0.insert("test", Arc::new(Rotating(calls.clone())));
        let resolver = bound.resolver(None);
        let session = SessionId::new();
        for n in 0..2 {
            assert_eq!(
                resolver
                    .get_connection_token(session, "test")
                    .await
                    .unwrap(),
                Some(format!("key-{n}"))
            )
        }
        let mut failing = Connections::default();
        failing.0.insert("test", Arc::new(Failing));
        let resolver = failing.resolver(Some(resolver));
        let error = resolver
            .get_connection_token(session, "test")
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("vault-secret-sentinel"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(
            !format!("{:?}", ApiKey::new("static-secret-sentinel"))
                .contains("static-secret-sentinel")
        );
    }
}
