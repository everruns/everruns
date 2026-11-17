//! MCP servers for orgs on the data plane (step 11.2).
//!
//! An agent's MCP servers come with its definition. The control plane
//! never puts a credential in a definition: a header that carries one holds
//! a `${BINDING}` placeholder, and the value is the org's secret
//! [`mcp_binding_secret`]; a server that acts as the agent's service
//! identity names an `mcp_oauth:<id>` connection (`oauth_provider_id`)
//! whose token secret is an OAuth token set. Both are opened per call, only on
//! the executor that runs the session's turn, for the session's own org.
//!
//! Decisions:
//! - Placeholders are taken out of the definition before the agent is
//!   built ([`take_bindings`]) and filled per call by [`AgentMcpAuth`]:
//!   nodes build the same agents executors run but never open secrets.
//! - A placeholder with no secret behind it is sent as written, as the
//!   platform's workers send headers.
//! - Opened values are kept for [`KEEP`] (a rotated value reaches running
//!   sessions within that), token sets until they near expiry.
//! - A token set within [`REFRESH_SKEW_S`] of its expiry is renewed here,
//!   under the directory cell's lease (`secrets.rs`), and written back as
//!   the secret's next revision: every executor then uses the new token,
//!   and the control plane gets it on the feed.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use everruns::hosting::services::{McpAuthProvider, McpAuthRequest, McpCredential};
use everruns_contracts::typed_id::SessionId;
use everruns_core::mcp::oauth::{OAuthClient, TokenSet};
use parking_lot::Mutex;

use crate::providers::OrgMaterial;
use crate::secrets::{SecretOp, SecretReply};

pub use everruns_contracts::dataplane::{MCP_OAUTH, mcp_binding_secret};

/// How long an opened binding value is reused.
const KEEP: Duration = Duration::from_secs(30);
/// How far ahead of its expiry a token is renewed.
pub const REFRESH_SKEW_S: i64 = 60;
/// How long a call waits for another executor's refresh.
const WAIT_FOR_REFRESH: Duration = Duration::from_secs(10);

/// What an agent's MCP servers take from the org per call.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Bindings {
    /// Header placeholders: server name to (header, binding name).
    pub headers: BTreeMap<String, Vec<(String, String)>>,
    /// Servers that act as the agent's service identity: server name to
    /// the `mcp_oauth` connection whose token they send.
    pub oauth: BTreeMap<String, String>,
}

impl Bindings {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.oauth.is_empty()
    }
}

/// Take every `${BINDING}` header and `mcp_oauth` connection out of
/// `servers`, to be filled per call. Environment placeholders go too: a
/// cell runs no local-process servers.
///
/// Decision: a server whose token comes from a connection is left with no
/// auth mode of its own. The token is sent from the first call, and the
/// runtime never starts an interactive sign-in a data plane cannot finish.
pub fn take_bindings(servers: &mut everruns_core::ScopedMcpServers) -> Bindings {
    let mut bindings = Bindings::default();
    for (name, server) in servers.iter_mut() {
        server
            .env
            .retain(|_, value| !everruns_core::agent_package::binding(value));
        if let Some(id) = server
            .oauth_provider_id
            .take_if(|id| id.starts_with(&format!("{MCP_OAUTH}:")))
        {
            server.auth_mode = everruns_core::McpServerAuthMode::None;
            bindings.oauth.insert(name.clone(), id);
        }
        let mut taken: Vec<(String, String)> = server
            .headers
            .iter()
            .filter(|(_, value)| everruns_core::agent_package::binding(value))
            .map(|(header, value)| (header.clone(), value[2..value.len() - 1].to_owned()))
            .collect();
        if taken.is_empty() {
            continue;
        }
        taken.sort();
        server.headers.retain(|header, _| {
            !taken
                .iter()
                .any(|(h, _)| h.eq_ignore_ascii_case(header.as_str()))
        });
        bindings.headers.insert(name.clone(), taken);
    }
    bindings
}

/// A secret as an executor opened it, with what it needs to write the
/// next revision.
pub struct OpenedSecret {
    pub revision: u64,
    pub plain: Vec<u8>,
    pub data_key: everruns_contracts::dataplane_seal::Key,
    pub data_key_id: String,
}

type Kept<T> = Mutex<HashMap<(String, String), (Instant, T)>>;

/// MCP credentials of the orgs whose turns run here.
pub struct OrgMcp {
    material: OnceLock<Weak<dyn OrgMaterial>>,
    egress: Arc<dyn everruns_core::EgressService>,
    /// Who leases a refresh: this process.
    holder: String,
    bindings: Kept<Option<String>>,
    tokens: Kept<TokenSet>,
    /// Time spent finding a call's credentials, by `kind`.
    pub resolve: crate::prom::Histograms,
    /// Refreshes, by `outcome`.
    pub refreshes: crate::prom::Counters,
}

impl OrgMcp {
    pub fn new(egress: Arc<dyn everruns_core::EgressService>) -> Self {
        Self {
            material: OnceLock::new(),
            egress,
            holder: format!("mcp-{}", uuid::Uuid::new_v4().simple()),
            bindings: Mutex::default(),
            tokens: Mutex::default(),
            resolve: Default::default(),
            refreshes: Default::default(),
        }
    }

    /// Token refreshes go out through the runtime's own egress.
    pub fn from_env() -> Self {
        Self::new(Arc::new(
            everruns_core::host::DirectEgressService::for_runtime_traffic_from_env(),
        ))
    }

    /// Resolve through `material` from now on.
    pub fn bind(&self, material: Weak<dyn OrgMaterial>) {
        let _ = self.material.set(material);
    }

    fn material(&self) -> anyhow::Result<Arc<dyn OrgMaterial>> {
        self.material
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| anyhow::anyhow!("this node does not open org secrets"))
    }

    /// `org`'s value for `binding`, if it has one.
    async fn binding(&self, org: &str, binding: &str) -> anyhow::Result<Option<String>> {
        let slot = (org.to_owned(), binding.to_owned());
        if let Some((at, value)) = self.bindings.lock().get(&slot)
            && at.elapsed() < KEEP
        {
            return Ok(value.clone());
        }
        let opened = self
            .material()?
            .open_secret(org, &mcp_binding_secret(binding))
            .await?;
        let value = opened
            .map(|s| String::from_utf8(s.plain))
            .transpose()
            .map_err(|_| anyhow::anyhow!("MCP credential {binding} is not text"))?;
        self.bindings
            .lock()
            .insert(slot, (Instant::now(), value.clone()));
        Ok(value)
    }

    /// The token set of `org`'s connection `id`, renewed when it nears
    /// expiry. `None`: the org has no such grant.
    pub async fn token(&self, org: &str, id: &str) -> anyhow::Result<Option<TokenSet>> {
        let slot = (org.to_owned(), id.to_owned());
        let fresh = |tokens: &TokenSet| !tokens.needs_refresh(chrono::Utc::now(), REFRESH_SKEW_S);
        if let Some((at, tokens)) = self.tokens.lock().get(&slot)
            && at.elapsed() < KEEP
            && fresh(tokens)
        {
            return Ok(Some(tokens.clone()));
        }
        let material = self.material()?;
        let Some(connection) = material.connection(org, id).await? else {
            return Ok(None);
        };
        anyhow::ensure!(
            connection.kind == MCP_OAUTH,
            "connection {id} is not an MCP OAuth grant"
        );
        let secret_id = connection.token_secret_id;
        let started = Instant::now();
        loop {
            let Some(opened) = material.open_secret(org, &secret_id).await? else {
                return Ok(None);
            };
            let tokens: TokenSet = serde_json::from_slice(&opened.plain)
                .map_err(|_| anyhow::anyhow!("connection {id} holds no OAuth token set"))?;
            // An expired set with nothing to renew it is still handed on:
            // the server's 401 says to connect again.
            if fresh(&tokens) || tokens.refresh_token.is_none() {
                self.tokens
                    .lock()
                    .insert(slot, (Instant::now(), tokens.clone()));
                return Ok(Some(tokens));
            }
            let lease = SecretOp::Lease {
                secret_id: secret_id.clone(),
                holder: self.holder.clone(),
                revision: opened.revision,
            };
            match material.secret_op(org, lease).await? {
                SecretReply::Leased { .. } => {
                    let renewed = self.refresh(org, &secret_id, &opened, &tokens).await?;
                    self.tokens
                        .lock()
                        .insert(slot, (Instant::now(), renewed.clone()));
                    return Ok(Some(renewed));
                }
                SecretReply::Missing => return Ok(None),
                SecretReply::Held { .. } if started.elapsed() > WAIT_FOR_REFRESH => {
                    anyhow::bail!("connection {id} is being refreshed elsewhere; try again")
                }
                SecretReply::Held { .. } => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                // Another executor refreshed it: read its revision.
                SecretReply::Changed { .. } | SecretReply::Stored { .. } => {}
            }
        }
    }

    /// Renew `tokens` at their token endpoint and store the result as the
    /// secret's next revision.
    async fn refresh(
        &self,
        org: &str,
        secret_id: &str,
        opened: &OpenedSecret,
        tokens: &TokenSet,
    ) -> anyhow::Result<TokenSet> {
        let client = OAuthClient::new(self.egress.as_ref(), everruns_core::EgressRequestKind::Mcp);
        let renewed = match client.refresh(tokens).await {
            Ok(renewed) => renewed,
            Err(error) => {
                self.refreshes.inc(&[("outcome", "failed")]);
                return Err(anyhow::anyhow!("renewing the MCP OAuth token: {error}"));
            }
        };
        let plain = serde_json::to_vec(&renewed)?;
        let revision = opened.revision + 1;
        let sealed = everruns_contracts::dataplane_seal::seal(
            &opened.data_key,
            &opened.data_key_id,
            org,
            secret_id,
            revision,
            &plain,
            everruns_contracts::dataplane_seal::random_nonce(),
        )?;
        let write = SecretOp::Refreshed {
            holder: self.holder.clone(),
            secret: sealed,
        };
        match self.material()?.secret_op(org, write).await? {
            SecretReply::Stored { .. } => {
                self.refreshes.inc(&[("outcome", "stored")]);
                Ok(renewed)
            }
            // Lost the lease meanwhile (it ran out): the provider may have
            // spent the old refresh token, but the stored revision is the
            // one every executor reads, so it stands.
            other => {
                self.refreshes.inc(&[("outcome", "lost")]);
                anyhow::bail!("the refreshed MCP token was not stored: {other:?}")
            }
        }
    }

    /// An agent's view: its header placeholders filled from its session's
    /// org.
    pub fn for_agent(self: &Arc<Self>, bindings: Bindings) -> AgentMcpAuth {
        AgentMcpAuth {
            org: self.clone(),
            bindings,
        }
    }

    async fn credential(
        &self,
        bindings: &Bindings,
        request: &McpAuthRequest<'_>,
    ) -> anyhow::Result<Option<McpCredential>> {
        let Some(session) = request.session_id else {
            return Ok(None);
        };
        let material = self.material()?;
        let Some(org) = material.tenant(SessionId::from_uuid(session)) else {
            return Ok(None);
        };
        let mut credential = McpCredential::default();
        let mut found = false;
        for (header, binding) in bindings
            .headers
            .get(request.server_name)
            .into_iter()
            .flatten()
        {
            let started = Instant::now();
            let value = self
                .binding(&org, binding)
                .await?
                .unwrap_or_else(|| format!("${{{binding}}}"));
            self.resolve
                .observe(&[("kind", "header")], started.elapsed());
            found = true;
            if header.eq_ignore_ascii_case("authorization") {
                credential.authorization = Some(value);
            } else {
                credential.headers.insert(header.clone(), value);
            }
        }
        if let Some(id) = bindings.oauth.get(request.server_name) {
            let started = Instant::now();
            let tokens = self.token(&org, id).await?;
            self.resolve
                .observe(&[("kind", "oauth")], started.elapsed());
            if let Some(tokens) = tokens {
                credential.authorization = Some(tokens.authorization_value());
                found = true;
            }
        }
        Ok(found.then_some(credential))
    }
}

/// MCP credentials of one agent's servers, for whichever org's session
/// calls them.
pub struct AgentMcpAuth {
    org: Arc<OrgMcp>,
    bindings: Bindings,
}

#[async_trait::async_trait]
impl McpAuthProvider for AgentMcpAuth {
    async fn authorization(
        &self,
        request: &McpAuthRequest<'_>,
    ) -> anyhow::Result<Option<McpCredential>> {
        self.org.credential(&self.bindings, request).await
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
