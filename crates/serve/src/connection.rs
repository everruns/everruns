//! Connections and secrets.
//!
//! A `#[connection]` returns any value; tools reach it with
//! `cx.connection::<T>()`, so credentials live in the connection and never in
//! the model context. [`McpServer`] is the one connection serve understands
//! itself: it is attached to every agent as an MCP server.
//!
//! Secrets are named, not read, at declaration time. The manifest lists every
//! [`Secret::named`] a connection or channel mentions, so the host can ask for
//! missing ones before the first deploy.

use std::collections::BTreeSet;
use std::sync::Mutex;

use anyhow::anyhow;

/// Every secret name mentioned while the app was being discovered.
static DECLARED: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

pub(crate) fn declared_secrets() -> BTreeSet<&'static str> {
    DECLARED.lock().map(|set| set.clone()).unwrap_or_default()
}

/// A secret provided by the host as an environment variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Secret {
    name: &'static str,
}

impl Secret {
    /// Declare a secret. Declaring records it for the manifest; nothing is
    /// read until [`value`](Self::value).
    pub fn named(name: &'static str) -> Self {
        if let Ok(mut set) = DECLARED.lock() {
            set.insert(name);
        }
        Self { name }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The value, from the environment the host prepared.
    pub fn value(&self) -> crate::Result<String> {
        std::env::var(self.name).map_err(|_| {
            anyhow!(
                "secret `{}` is not set; the host provides it as an environment variable",
                self.name
            )
        })
    }

    pub fn is_present(&self) -> bool {
        std::env::var_os(self.name).is_some_and(|value| !value.is_empty())
    }
}

/// An MCP server connection, attached to every agent.
#[derive(Clone, Debug)]
pub struct McpServer {
    pub(crate) url: String,
    pub(crate) auth: Option<Secret>,
}

impl McpServer {
    /// A streamable-HTTP (or SSE) MCP endpoint.
    pub fn http(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            auth: None,
        }
    }

    /// Send `Authorization: Bearer <secret>`.
    pub fn auth(mut self, secret: Secret) -> Self {
        self.auth = Some(secret);
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// The everruns server config, or why it cannot be attached right now.
    pub(crate) fn to_everruns(&self, name: &str) -> crate::Result<everruns::McpServer> {
        let mut server = everruns::McpServer::http(name, &self.url);
        if let Some(secret) = self.auth {
            server = server.header("Authorization", format!("Bearer {}", secret.value()?));
        }
        Ok(server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaring_a_secret_records_it_without_reading_it() {
        let secret = Secret::named("SERVE_TEST_DECLARED_ONLY");
        assert!(declared_secrets().contains("SERVE_TEST_DECLARED_ONLY"));
        assert!(!secret.is_present());
        let err = secret.value().unwrap_err().to_string();
        assert!(err.contains("SERVE_TEST_DECLARED_ONLY"), "{err}");
    }

    #[test]
    fn mcp_without_its_secret_is_not_attachable() {
        let server = McpServer::http("https://example.test/mcp")
            .auth(Secret::named("SERVE_TEST_MISSING_TOKEN"));
        assert!(server.to_everruns("linear").is_err());
        assert!(
            McpServer::http("https://example.test/mcp")
                .to_everruns("x")
                .is_ok()
        );
    }
}
