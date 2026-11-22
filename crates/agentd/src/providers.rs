//! Model providers for orgs on the data plane (design.md, step 8): an org's
//! provider is an `llm_provider` connection on its config stream, and its
//! API key a sealed secret, opened only on the executor that runs a turn.
//!
//! Decisions:
//! - The connection is `llm:<provider key>`, the key being the provider's
//!   normalized name, the same key an agent's `model.provider` names. Its
//!   settings carry what a driver needs besides the key (`provider_type`,
//!   `base_url`, `api`, `request_options`); its token secret is the key.
//! - A turn's provider comes from its session's org, never from another
//!   org's connection under the same name.
//! - Opened material is kept in memory for [`KEEP`], so a turn's steps do
//!   not go back to the directory shard, and a key rotation reaches running
//!   sessions within that time. It is never written anywhere.
//! - A lookup without a session (the Engine checks, when it builds an
//!   agent, that something serves its provider) answers a provider with no
//!   key and no endpoint: whether the org has that provider is the control
//!   plane's check when it publishes the agent, and a turn whose org lacks
//!   it fails with "not configured". So nodes, which never open secrets,
//!   build the same agents executors run, and nothing outside a session
//!   can call a model with an org's key.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use everruns::hosting::services::{
    AgentLoopError, DriverRegistry, HostProviders, ProviderConfig, ProviderStore, Result,
};
use everruns_contracts::dataplane::Connection;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime_provider::ProviderKey;
use everruns_contracts::typed_id::{ModelId, SessionId};
use parking_lot::Mutex;

pub use everruns_contracts::dataplane::{
    LLM_PROVIDER as KIND, LlmProviderSettings as Settings, llm_provider_connection as connection_id,
};

/// How long opened provider material is reused.
const KEEP: Duration = Duration::from_secs(30);

pub(crate) const MODEL_NOT_CONFIGURED: &str =
    "Connect a provider and select a chat model in Settings → Models.";

/// The driver configuration of an `llm_provider` connection.
pub trait SettingsExt {
    /// With `api_key` when it was opened.
    fn config(&self, api_key: Option<String>) -> ProviderConfig;
}

impl SettingsExt for Settings {
    fn config(&self, api_key: Option<String>) -> ProviderConfig {
        // Unknown driver names parse as external drivers.
        let driver = self
            .provider_type
            .parse()
            .unwrap_or_else(|never| match never {});
        let mut config = ProviderConfig::for_provider(ProviderKey::new(&self.provider_key), driver);
        config.api_key = api_key;
        config.base_url = self.base_url.clone();
        config.api = self.api.clone();
        config.request_options = self.request_options.clone();
        config
    }
}

/// What the machine running a turn knows about orgs.
#[async_trait::async_trait]
pub trait OrgMaterial: Send + Sync {
    /// The org a session belongs to.
    fn tenant(&self, session: SessionId) -> Option<String>;
    /// `org`'s connection `id`.
    async fn connection(&self, org: &str, id: &str) -> anyhow::Result<Option<Connection>>;
    /// `org`'s secret, opened.
    async fn secret(&self, org: &str, secret_id: &str) -> anyhow::Result<Vec<u8>>;
    /// `org`'s secret, opened, with its revision and the org's data key;
    /// `None` when the org has no such secret.
    async fn open_secret(
        &self,
        org: &str,
        secret_id: &str,
    ) -> anyhow::Result<Option<crate::mcp::OpenedSecret>> {
        let _ = (org, secret_id);
        anyhow::bail!("this node does not open org secrets")
    }
    /// Whether session secrets are sealed here: false on a standalone shard,
    /// which has no data plane key because no control plane wraps keys.
    fn seals(&self) -> bool {
        false
    }
    /// `org`'s data key, unwrapped, with its id: what session secrets are
    /// sealed with (`hosting_ports.rs`); `None` where nothing seals them (a
    /// standalone shard).
    async fn data_key(
        &self,
        org: &str,
    ) -> anyhow::Result<Option<(everruns_contracts::dataplane_seal::Key, String)>> {
        let _ = org;
        anyhow::bail!("this node does not open session secrets")
    }
    /// Lease or write one of `org`'s secrets in the directory shard
    /// (`secrets.rs`).
    async fn secret_op(
        &self,
        org: &str,
        op: crate::secrets::SecretOp,
    ) -> anyhow::Result<crate::secrets::SecretReply> {
        let _ = (org, op);
        anyhow::bail!("this node does not refresh org secrets")
    }
    /// The person whose message opened the turn `session` has open, with
    /// that message, from the session's own record (`senders.rs`); `None`
    /// when no person did or no turn is open.
    fn turn_actor(
        &self,
        session: SessionId,
    ) -> Option<everruns_contracts::runtime::capabilities::injected::SessionActor> {
        let _ = session;
        None
    }
    /// The person who sent `message` in `session`, with that message, from
    /// the session's own record (`senders.rs`); `None` when no person did.
    fn message_actor(
        &self,
        session: SessionId,
        message: everruns_contracts::typed_id::MessageId,
    ) -> Option<everruns_contracts::runtime::capabilities::injected::SessionActor> {
        let _ = (session, message);
        None
    }
    /// The URL of the control plane's server `name`
    /// (`injected::CONTROLLER_SERVERS`) that `session`'s create brought;
    /// `None` without one, or for a session not known here.
    fn controller_url(&self, session: SessionId, name: &str) -> Option<String> {
        let _ = (session, name);
        None
    }
    /// The data plane's key, which session tokens for injected
    /// capabilities are signed with a key derived from
    /// ([`session_token_pkcs8`](everruns_contracts::dataplane_seal::session_token_pkcs8));
    /// `None` where no control plane wraps keys (a standalone shard).
    fn data_plane_key(&self) -> Option<everruns_contracts::dataplane_seal::Key> {
        None
    }
    /// The orgs' MCP credentials, session tokens included; `None` on a
    /// node that calls no control plane server (`memory.rs`).
    fn mcp(&self) -> Option<Arc<crate::mcp::OrgMcp>> {
        None
    }
    /// Whether [`lookup`](Self::lookup) reaches a control plane: false on
    /// a standalone shard, whose tools then get the defaults a host without
    /// org settings gives.
    fn looks_up(&self) -> bool {
        false
    }
    /// The host calls `session`'s agent declares (its capabilities'
    /// `host_calls`); `None` when the session is not known here, and then
    /// nothing is checked.
    fn declared_host_calls(&self, session: SessionId) -> Option<BTreeSet<&'static str>> {
        let _ = session;
        None
    }
    /// A session made host call `call`, which its capabilities do not
    /// declare (counted for the matrix's declarations to be checked live).
    fn undeclared_host_call(&self, call: &str) {
        let _ = call;
    }
    /// Make one of `org`'s host calls, named by its v1 worker command
    /// (`lookups.rs`).
    async fn lookup(
        &self,
        org: &str,
        command: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<everruns_contracts::host_calls::HostCallAnswer> {
        let _ = (org, command, params);
        anyhow::bail!("this node does not reach a control plane")
    }
}

/// (org, key) -> the provider, opened, and when.
type Opened = HashMap<(String, String), (Instant, Option<ProviderConfig>)>;

/// The providers of the orgs whose turns run here.
#[derive(Default)]
pub struct OrgProviders {
    material: OnceLock<Weak<dyn OrgMaterial>>,
    opened: Mutex<Opened>,
}

impl OrgProviders {
    /// Resolve through `material` from now on.
    pub fn bind(&self, material: Weak<dyn OrgMaterial>) {
        let _ = self.material.set(material);
    }

    /// As the Engine takes them, with every driver this build has.
    pub fn host_providers(self: &Arc<Self>) -> HostProviders {
        let mut drivers = DriverRegistry::new();
        everruns_drivers::register_drivers(&mut drivers);
        // Decision: the worker's deterministic LlmSim driver too, so an org's
        // LlmSim provider (a self-hosted install's keyless first agent, the
        // Compose smoke test) answers on a shard as it does on a worker.
        everruns_llmsim::register_driver(&mut drivers);
        HostProviders::shared(self.clone(), drivers)
    }

    fn material(&self) -> Option<Arc<dyn OrgMaterial>> {
        self.material.get().and_then(Weak::upgrade)
    }

    /// `org`'s provider `key`, opened.
    async fn of(&self, org: &str, key: &ProviderKey) -> Result<Option<ProviderConfig>> {
        let slot = (org.to_owned(), key.as_str().to_owned());
        if let Some((at, config)) = self.opened.lock().get(&slot)
            && at.elapsed() < KEEP
        {
            return Ok(config.clone());
        }
        let Some(material) = self.material() else {
            return Ok(None);
        };
        let config = match material
            .connection(org, &connection_id(key.as_str()))
            .await
            .map_err(unavailable)?
        {
            Some(connection) if connection.kind == KIND => {
                let settings: Settings =
                    serde_json::from_value(connection.settings).map_err(unavailable)?;
                let secret = material
                    .secret(org, &connection.token_secret_id)
                    .await
                    .map_err(unavailable)?;
                let key = String::from_utf8(secret).map_err(unavailable)?;
                Some(settings.config((!key.is_empty()).then_some(key)))
            }
            _ => None,
        };
        self.opened
            .lock()
            .insert(slot, (Instant::now(), config.clone()));
        Ok(config)
    }
}

fn unavailable(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::config(format!("the org's model provider is unavailable: {error}"))
}

#[async_trait::async_trait]
impl ProviderStore for OrgProviders {
    async fn get_model_spec(&self, _model_id: ModelId) -> Result<Option<ModelSpec>> {
        Ok(None)
    }

    async fn get_default_model_spec(&self) -> Result<Option<ModelSpec>> {
        Ok(None)
    }

    async fn get_provider_config_for_session(
        &self,
        provider: &ProviderKey,
        session: SessionId,
    ) -> Result<Option<ProviderConfig>> {
        if provider.as_str().is_empty() {
            return Err(AgentLoopError::config(MODEL_NOT_CONFIGURED));
        }
        let Some(org) = self.material().and_then(|m| m.tenant(session)) else {
            return Ok(None);
        };
        self.of(&org, provider).await
    }

    async fn get_provider_config(&self, provider: &ProviderKey) -> Result<Option<ProviderConfig>> {
        let unopened = Settings {
            provider_key: provider.as_str().to_owned(),
            provider_type: format!("org:{provider}"),
            ..Settings::default()
        };
        Ok(Some(unopened.config(None)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unconfigured_model_never_resolves_provider_credentials() {
        let providers = OrgProviders::default();
        let error = providers
            .get_provider_config_for_session(&ProviderKey::new(""), SessionId::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains(MODEL_NOT_CONFIGURED));
    }

    struct Orgs;

    #[async_trait::async_trait]
    impl OrgMaterial for Orgs {
        fn tenant(&self, _session: SessionId) -> Option<String> {
            Some("org-a".into())
        }

        async fn connection(&self, org: &str, id: &str) -> anyhow::Result<Option<Connection>> {
            Ok(
                (org == "org-a" && id == "llm:openrouter").then(|| Connection {
                    connection_id: id.into(),
                    kind: KIND.into(),
                    settings: serde_json::json!({
                        "provider_key": "openrouter",
                        "provider_type": "openrouter",
                        "base_url": "https://openrouter.ai/api/v1",
                    }),
                    token_secret_id: id.into(),
                    refresh_owner: Default::default(),
                }),
            )
        }

        async fn secret(&self, org: &str, _secret_id: &str) -> anyhow::Result<Vec<u8>> {
            Ok(format!("key-of-{org}").into_bytes())
        }
    }

    #[tokio::test]
    async fn a_session_gets_its_own_orgs_key_and_nothing_else_gets_a_key() {
        let providers = Arc::new(OrgProviders::default());
        let material: Arc<dyn OrgMaterial> = Arc::new(Orgs);
        providers.bind(Arc::downgrade(&material));
        let key = ProviderKey::new("OpenRouter");
        let config = providers
            .get_provider_config_for_session(&key, SessionId::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(config.api_key.as_deref(), Some("key-of-org-a"));
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        let shape = providers.get_provider_config(&key).await.unwrap().unwrap();
        assert_eq!((shape.api_key, shape.base_url), (None, None));
        let missing = ProviderKey::new("anthropic");
        assert!(
            providers
                .get_provider_config_for_session(&missing, SessionId::new())
                .await
                .unwrap()
                .is_none()
        );
    }
}
