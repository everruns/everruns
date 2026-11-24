//! Installation and transport scoped Channelkit clients.
use channelkit::{ConversationKey, MessageRef, Provider, http::SharedTransport};
use everruns_contracts::runtime::{
    channel::{DeliveryContext, DeliveryResult},
    egress::EgressService,
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
pub const INSTALLATION_ID: &str = "slack_installation_id";
pub const TEAM_ID: &str = "slack_team_id";
const MAX_CLIENTS: usize = 128;
const CLIENT_TTL: Duration = Duration::from_secs(300);
#[derive(Clone, Copy, PartialEq, Eq)]
enum TransportIdentity {
    Default,
    Scoped(usize),
}
#[derive(PartialEq, Eq)]
struct ClientKey {
    installation: String,
    team: String,
    api: String,
    credentials: [u8; 32],
    transport: TransportIdentity,
}
#[derive(PartialEq, Eq)]
struct IdentityKey {
    installation: String,
    api: String,
    credential: [u8; 32],
    transport: TransportIdentity,
}
type IdentityCell = Arc<tokio::sync::OnceCell<channelkit_slack::SlackIdentity>>;
struct IdentityEntry {
    key: IdentityKey,
    created: Instant,
    cell: IdentityCell,
    // Keep the scoped service alive even when a later team check refuses the
    // client. Its pointer cannot be reused while its identity remains cached.
    _transport: SharedTransport,
}
fn credential_revision(token: Option<&str>, secret: Option<&str>) -> [u8; 32] {
    let mut hash = Sha256::new();
    for value in [token, secret] {
        hash.update([u8::from(value.is_some())]);
        let value = value.unwrap_or_default();
        hash.update(value.len().to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.finalize().into()
}
pub struct SlackAdapterFactory {
    transport: SharedTransport,
    clients: Mutex<VecDeque<(ClientKey, Instant, Arc<channelkit_slack::Slack>)>>,
    identities: Mutex<VecDeque<IdentityEntry>>,
}
impl SlackAdapterFactory {
    pub fn new(transport: SharedTransport) -> Self {
        Self {
            transport,
            clients: Mutex::new(VecDeque::new()),
            identities: Mutex::new(VecDeque::new()),
        }
    }
    pub fn get(
        &self,
        installation: &str,
        team: &str,
        token: Option<&str>,
        secret: Option<&str>,
        api: &str,
    ) -> channelkit::Result<Arc<channelkit_slack::Slack>> {
        self.get_with_transport(
            installation,
            team,
            token,
            secret,
            api,
            TransportIdentity::Default,
            self.transport.clone(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn get_with_transport(
        &self,
        installation: &str,
        team: &str,
        token: Option<&str>,
        secret: Option<&str>,
        api: &str,
        transport_id: TransportIdentity,
        transport: SharedTransport,
    ) -> channelkit::Result<Arc<channelkit_slack::Slack>> {
        let key = ClientKey {
            installation: installation.into(),
            team: team.into(),
            api: api.into(),
            credentials: credential_revision(token, secret),
            transport: transport_id,
        };
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| channelkit::Error::Transport)?;
        clients.retain(|(_, created, _)| created.elapsed() < CLIENT_TTL);
        if let Some((_, _, client)) = clients.iter().find(|(existing, _, _)| existing == &key) {
            return Ok(client.clone());
        }
        let client = match (token, secret) {
            (Some(token), Some(secret)) => {
                channelkit_slack::Slack::new(installation, team, token, secret)
            }
            (Some(token), None) => {
                channelkit_slack::Slack::new_socket_mode(installation, team, token)
            }
            (None, Some(secret)) => {
                channelkit_slack::Slack::new_ingress(installation, team, secret)
            }
            _ => Err(channelkit::Error::InvalidInput("Slack credentials")),
        }?
        .transport(transport)
        .api_origin(api)?;
        let client = Arc::new(client);
        if clients.len() >= MAX_CLIENTS {
            clients.pop_front();
        }
        clients.push_back((key, Instant::now(), client.clone()));
        Ok(client)
    }
    pub async fn identity(
        &self,
        installation: &str,
        token: &str,
        api: &str,
    ) -> channelkit::Result<channelkit_slack::SlackIdentity> {
        self.identity_with_transport(
            installation,
            token,
            api,
            TransportIdentity::Default,
            self.transport.clone(),
        )
        .await
    }
    async fn identity_with_transport(
        &self,
        installation: &str,
        token: &str,
        api: &str,
        transport_id: TransportIdentity,
        transport: SharedTransport,
    ) -> channelkit::Result<channelkit_slack::SlackIdentity> {
        if installation.is_empty()
            || installation.len() > 256
            || token.is_empty()
            || token.len() > 4096
        {
            return Err(channelkit::Error::InvalidInput(
                "Slack installation or credential bounds",
            ));
        }
        let key = IdentityKey {
            installation: installation.into(),
            api: api.into(),
            credential: credential_revision(Some(token), None),
            transport: transport_id,
        };
        let cell = {
            let mut entries = self
                .identities
                .lock()
                .map_err(|_| channelkit::Error::Transport)?;
            entries.retain(|entry| entry.created.elapsed() < CLIENT_TTL);
            if let Some(entry) = entries.iter().find(|entry| entry.key == key) {
                entry.cell.clone()
            } else {
                if entries.len() >= MAX_CLIENTS {
                    entries.pop_front();
                }
                let cell = Arc::new(tokio::sync::OnceCell::new());
                entries.push_back(IdentityEntry {
                    key,
                    created: Instant::now(),
                    cell: cell.clone(),
                    _transport: transport.clone(),
                });
                cell
            }
        };
        cell.get_or_try_init(|| channelkit_slack::inspect_token(token, api, transport))
            .await
            .cloned()
    }
    fn scoped_transport(
        &self,
        egress: Option<Arc<dyn EgressService>>,
    ) -> (TransportIdentity, SharedTransport) {
        match egress {
            Some(egress) => {
                let identity = Arc::as_ptr(&egress) as *const () as usize;
                (
                    TransportIdentity::Scoped(identity),
                    Arc::new(crate::channelkit_bridge::EgressTransport::new(egress)),
                )
            }
            None => (TransportIdentity::Default, self.transport.clone()),
        }
    }
    /// Inspect identity through the same host-scoped transport used for writes.
    pub async fn scoped_identity(
        &self,
        installation: &str,
        token: &str,
        api: &str,
        egress: Option<Arc<dyn EgressService>>,
    ) -> channelkit::Result<channelkit_slack::SlackIdentity> {
        let (identity, transport) = self.scoped_transport(egress);
        self.identity_with_transport(installation, token, api, identity, transport)
            .await
    }

    /// Provider reads and writes use the same host service and credential revision.
    pub async fn for_credentials(
        &self,
        installation: &str,
        team: Option<&str>,
        token: &str,
        api: &str,
        egress: Option<Arc<dyn EgressService>>,
    ) -> channelkit::Result<Arc<channelkit_slack::Slack>> {
        let (transport_id, transport) = self.scoped_transport(egress);
        let actual = self
            .identity_with_transport(installation, token, api, transport_id, transport.clone())
            .await?;
        if team
            .filter(|team| !team.is_empty())
            .is_some_and(|team| team != actual.team_id)
        {
            return Err(channelkit::Error::Scope);
        }
        self.get_with_transport(
            installation,
            &actual.team_id,
            Some(token),
            None,
            api,
            transport_id,
            transport,
        )
    }
    pub async fn for_scoped_context(
        &self,
        context: &DeliveryContext,
        api: &str,
        egress: Option<Arc<dyn EgressService>>,
    ) -> channelkit::Result<Arc<channelkit_slack::Slack>> {
        let installation =
            context
                .extra
                .get(INSTALLATION_ID)
                .ok_or(channelkit::Error::InvalidInput(
                    "Slack installation identity",
                ))?;
        let team = context.extra.get(TEAM_ID).filter(|team| !team.is_empty());
        let recipient = context
            .extra
            .get(super::surfaces::SLACK_RECIPIENT_TEAM_ID)
            .filter(|team| !team.is_empty());
        if let (Some(team), Some(recipient)) = (team, recipient)
            && team != recipient
        {
            return Err(channelkit::Error::Scope);
        }
        self.for_credentials(
            installation,
            team.or(recipient).map(String::as_str),
            &context.auth_token,
            api,
            egress,
        )
        .await
    }
    pub async fn for_context(
        &self,
        context: &DeliveryContext,
        api: &str,
    ) -> channelkit::Result<Arc<channelkit_slack::Slack>> {
        self.for_scoped_context(context, api, context.egress.clone())
            .await
    }
}
pub fn direct_factory() -> channelkit::Result<&'static SlackAdapterFactory> {
    static FACTORY: OnceLock<Result<SlackAdapterFactory, ()>> = OnceLock::new();
    FACTORY
        .get_or_init(|| {
            channelkit::http::ReqwestTransport::new()
                .map(|transport| SlackAdapterFactory::new(Arc::new(transport)))
                .map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| channelkit::Error::Transport)
}
pub fn target(
    client: &channelkit_slack::Slack,
    context: &DeliveryContext,
) -> channelkit::Result<ConversationKey> {
    use channelkit::Adapter;
    ConversationKey::new(
        Provider::Slack,
        client.installation(),
        &context.channel_id,
        (!context.thread_ref.is_empty()).then(|| context.thread_ref.clone()),
    )
}
pub fn reference(
    client: &channelkit_slack::Slack,
    context: &DeliveryContext,
    id: &str,
) -> channelkit::Result<MessageRef> {
    if id.is_empty() || id.len() > 256 {
        return Err(channelkit::Error::InvalidInput("Slack message identifier"));
    }
    Ok(MessageRef {
        conversation: target(client, context)?,
        id: id.into(),
    })
}
pub fn message_key(reference: &MessageRef) -> String {
    format!("{}:{}", reference.conversation.channel, reference.id)
}
pub fn outcome(error: &channelkit::Error) -> DeliveryResult {
    crate::channelkit_bridge::delivery_outcome(
        error,
        |code| super::web_api::PERMANENT_SLACK_ERRORS.contains(&code),
        message_key,
    )
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
