//! A person's Poppy client: runs one [`PoppyCall`] at a time over the
//! protocol, keeping the person's accounts and conversations in a
//! [`PoppyStore`].
//!
//! Decision: the client owns the protocol flow (discovery, Sessions and their
//! renewal, cursors) and the store owns where state lives and how it is
//! sealed. The server stores it per session in Postgres, encrypted; a
//! Framework app can keep it in memory ([`InMemoryPoppyStore`]).
//!
//! Decision: one conversation per company per store. Starting a new one keeps
//! the Session it can renew when the company's issuer is unchanged.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::key::PoppyKey;
use super::protocol::{Company, PoppyAgent, SessionToken, company_host};
use super::{PoppyCall, PoppyInvoker};

/// Seconds before expiry a Session Token is renewed rather than used.
const TOKEN_MARGIN_SECS: i64 = 30;

/// Why a call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoppyError {
    /// The company refused, or could not be reached: a sentence for the model.
    Refused(String),
    /// The client's own state failed (storage, keys): not for the model.
    Internal(String),
}

impl std::fmt::Display for PoppyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(message) | Self::Internal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PoppyError {}

/// The person's pairwise identity at one company issuer: a random User ID
/// and its own DPoP key, so two companies cannot correlate them.
pub struct PoppyAccount {
    pub user_id: String,
    pub dpop: PoppyKey,
}

impl PoppyAccount {
    /// A fresh identity.
    pub fn generate() -> Result<Self, String> {
        Ok(Self {
            user_id: format!("usr_{}", uuid::Uuid::new_v4().simple()),
            dpop: PoppyKey::generate()?,
        })
    }
}

/// What a conversation with one company remembers between calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationState {
    pub name: Option<String>,
    pub token_endpoint: String,
    pub conversations: String,
    pub poppy_session_id: Option<String>,
    pub conversation_id: Option<String>,
    pub cursor: Option<String>,
}

/// A stored conversation: the company, its issuer, the state, and the
/// Session Token with its expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredConversation {
    pub domain: String,
    pub issuer: String,
    pub state: ConversationState,
    pub token: Option<(String, DateTime<Utc>)>,
}

/// Where a person's Poppy state lives. Errors are internal failures.
#[async_trait]
pub trait PoppyStore: Send + Sync {
    /// The person's account at `issuer`, created on first use.
    async fn account(&self, issuer: &str) -> Result<PoppyAccount, String>;
    /// The conversation held with the company at `domain`.
    async fn conversation(&self, domain: &str) -> Result<Option<StoredConversation>, String>;
    /// Store `conversation`, replacing the one held for its domain.
    async fn save_conversation(&self, conversation: &StoredConversation) -> Result<(), String>;
}

/// A [`PoppyStore`] in process memory, for one person. Nothing survives a
/// restart: a new run is a new person to every company.
#[derive(Default)]
pub struct InMemoryPoppyStore {
    accounts: parking_lot::Mutex<HashMap<String, (String, Vec<u8>)>>,
    conversations: parking_lot::Mutex<HashMap<String, StoredConversation>>,
}

impl InMemoryPoppyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PoppyStore for InMemoryPoppyStore {
    async fn account(&self, issuer: &str) -> Result<PoppyAccount, String> {
        let mut accounts = self.accounts.lock();
        if let Some((user_id, pkcs8)) = accounts.get(issuer) {
            return Ok(PoppyAccount {
                user_id: user_id.clone(),
                dpop: PoppyKey::from_pkcs8(pkcs8)?,
            });
        }
        let account = PoppyAccount::generate()?;
        accounts.insert(
            issuer.to_string(),
            (account.user_id.clone(), account.dpop.pkcs8().to_vec()),
        );
        Ok(account)
    }

    async fn conversation(&self, domain: &str) -> Result<Option<StoredConversation>, String> {
        Ok(self.conversations.lock().get(domain).cloned())
    }

    async fn save_conversation(&self, conversation: &StoredConversation) -> Result<(), String> {
        self.conversations
            .lock()
            .insert(conversation.domain.clone(), conversation.clone());
        Ok(())
    }
}

/// A person's Poppy client.
pub struct PoppyClient {
    agent: Arc<PoppyAgent>,
    store: Arc<dyn PoppyStore>,
}

/// A conversation ready to use: its state, account key and live token.
struct Live {
    conversation: StoredConversation,
    user_id: String,
    dpop: PoppyKey,
    token: String,
}

fn refused(message: String) -> PoppyError {
    PoppyError::Refused(message)
}

fn internal(message: String) -> PoppyError {
    PoppyError::Internal(message)
}

impl PoppyClient {
    pub fn new(agent: Arc<PoppyAgent>, store: Arc<dyn PoppyStore>) -> Self {
        Self { agent, store }
    }

    /// Run one call. `Ok` is the answer for the model.
    pub async fn run(&self, call: PoppyCall) -> Result<Value, PoppyError> {
        match call {
            PoppyCall::Discover { company } => {
                let company = self.agent.discover(&company).await.map_err(refused)?;
                Ok(company.summary())
            }
            PoppyCall::Send {
                company,
                text,
                message_id,
                new_conversation,
                wait_seconds,
            } => {
                self.send(&company, &text, &message_id, new_conversation, wait_seconds)
                    .await
            }
            PoppyCall::Read {
                company,
                wait_seconds,
            } => self.read(&company, wait_seconds).await,
            PoppyCall::Handoff { company } => self.action(&company, "handoff").await,
            PoppyCall::Close { company } => self.action(&company, "close").await,
        }
    }

    async fn save(&self, live: &Live) -> Result<(), PoppyError> {
        self.store
            .save_conversation(&live.conversation)
            .await
            .map_err(internal)
    }

    /// Start a Session (or renew the one held) and store its token.
    async fn refresh(&self, live: &mut Live) -> Result<(), PoppyError> {
        let state = &live.conversation.state;
        let company = Company {
            domain: live.conversation.domain.clone(),
            name: state.name.clone(),
            issuer: live.conversation.issuer.clone(),
            token_endpoint: state.token_endpoint.clone(),
            revocation_endpoint: None,
            conversations: Some(state.conversations.clone()),
            document: Value::Null,
        };
        let renewed = match &state.poppy_session_id {
            Some(id) => self
                .agent
                .start_session(&company, &live.user_id, &live.dpop, Some(id))
                .await
                .ok(),
            None => None,
        };
        let token: SessionToken = match renewed {
            Some(token) => token,
            None => self
                .agent
                .start_session(&company, &live.user_id, &live.dpop, None)
                .await
                .map_err(refused)?,
        };
        let expires = Utc::now() + Duration::seconds(token.expires_in.max(0));
        live.conversation.token = Some((token.access_token.clone(), expires));
        live.conversation.state.poppy_session_id = Some(token.session_id);
        live.token = token.access_token;
        self.save(live).await
    }

    /// The conversation with `company`, ready to use: discovered and with a
    /// Session on first use (or when `fresh`).
    async fn live(&self, company: &str, fresh: bool) -> Result<Live, PoppyError> {
        let domain = company_host(company).map_err(refused)?;
        let stored = self.store.conversation(&domain).await.map_err(internal)?;
        let conversation = match stored {
            Some(conversation) if !fresh => conversation,
            previous => {
                let discovered = self.agent.discover(&domain).await.map_err(refused)?;
                let conversations = discovered.conversations.clone().ok_or_else(|| {
                    refused(format!("{domain} offers no conversations with its agent"))
                })?;
                // A new conversation keeps the Session it can renew.
                let previous = previous.filter(|previous| previous.issuer == discovered.issuer);
                StoredConversation {
                    domain: domain.clone(),
                    issuer: discovered.issuer.clone(),
                    state: ConversationState {
                        name: discovered.name.clone(),
                        token_endpoint: discovered.token_endpoint.clone(),
                        conversations,
                        poppy_session_id: previous
                            .as_ref()
                            .and_then(|previous| previous.state.poppy_session_id.clone()),
                        conversation_id: None,
                        cursor: None,
                    },
                    token: previous.and_then(|previous| previous.token),
                }
            }
        };
        let account = self
            .store
            .account(&conversation.issuer)
            .await
            .map_err(internal)?;
        let token = conversation
            .token
            .as_ref()
            .filter(|(_, expires)| *expires - Duration::seconds(TOKEN_MARGIN_SECS) > Utc::now())
            .map(|(token, _)| token.clone())
            .unwrap_or_default();
        let mut live = Live {
            conversation,
            user_id: account.user_id,
            dpop: account.dpop,
            token,
        };
        if live.token.is_empty() {
            self.refresh(&mut live).await?;
        }
        Ok(live)
    }

    /// One authenticated request; on `401` the Session is renewed and the
    /// request sent once more.
    async fn authed(
        &self,
        live: &mut Live,
        method: &str,
        url: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value), PoppyError> {
        for attempt in 0..2 {
            let answer = match (method, body) {
                ("GET", _) => self.agent.read_events(url, &live.token, &live.dpop).await,
                (_, Some(message)) => {
                    self.agent
                        .post_message(url, &live.token, &live.dpop, message)
                        .await
                }
                (_, None) => self.agent.post_action(url, &live.token, &live.dpop).await,
            }
            .map_err(refused)?;
            if answer.0 == 401 && attempt == 0 {
                self.refresh(live).await?;
                continue;
            }
            return Ok(answer);
        }
        unreachable!("the loop returns on its second attempt")
    }

    fn conversation_url(live: &Live) -> Result<String, PoppyError> {
        let state = &live.conversation.state;
        match &state.conversation_id {
            Some(id) => Ok(format!("{}/{id}", state.conversations)),
            None => Err(refused(format!(
                "there is no conversation with {} yet; send a message first",
                live.conversation.domain
            ))),
        }
    }

    async fn send(
        &self,
        company: &str,
        text: &str,
        message_id: &str,
        new_conversation: bool,
        wait_seconds: u32,
    ) -> Result<Value, PoppyError> {
        let mut live = self.live(company, new_conversation).await?;
        let state = &live.conversation.state;
        // Without a cursor the answer reads from the start of the
        // conversation (spec 7.3), so a later message passes the last one.
        let mut query = format!("wait={wait_seconds}");
        if let Some(cursor) = state.conversation_id.as_ref().and(state.cursor.as_ref()) {
            query.push_str(&format!("&cursor={}", urlencoding::encode(cursor)));
        }
        let url = match &state.conversation_id {
            Some(id) => format!("{}/{id}/messages?{query}", state.conversations),
            None => format!("{}?{query}", state.conversations),
        };
        let message = json!({
            "id": message_id,
            "sender": "agent",
            "text": text,
            "context": { "user_available": false },
        });
        let (status, body) = self.authed(&mut live, "POST", &url, Some(&message)).await?;
        if !(200..300).contains(&status) {
            return Err(refused(company_error(
                &live.conversation.domain,
                "message",
                status,
                &body,
            )));
        }
        if let Some(id) = body.get("conversation_id").and_then(Value::as_str) {
            live.conversation.state.conversation_id = Some(id.to_string());
        }
        let messages = absorb(&mut live.conversation.state, &body);
        self.save(&live).await?;
        Ok(answer(&live, &body, messages))
    }

    async fn read(&self, company: &str, wait_seconds: u32) -> Result<Value, PoppyError> {
        let mut live = self.live(company, false).await?;
        let base = Self::conversation_url(&live)?;
        let mut url = format!("{base}/events?wait={wait_seconds}");
        if let Some(cursor) = &live.conversation.state.cursor {
            url.push_str(&format!("&cursor={}", urlencoding::encode(cursor)));
        }
        let (status, body) = self.authed(&mut live, "GET", &url, None).await?;
        if status == 410 {
            // The cursor fell out of the company's window: start over.
            live.conversation.state.cursor = None;
            self.save(&live).await?;
            return Err(refused(
                "the company no longer has those events; read again to start from what it still has"
                    .into(),
            ));
        }
        if status != 200 {
            return Err(refused(company_error(
                &live.conversation.domain,
                "read",
                status,
                &body,
            )));
        }
        let messages = absorb(&mut live.conversation.state, &body);
        self.save(&live).await?;
        Ok(answer(&live, &body, messages))
    }

    async fn action(&self, company: &str, action: &str) -> Result<Value, PoppyError> {
        let mut live = self.live(company, false).await?;
        let url = format!("{}/{action}", Self::conversation_url(&live)?);
        let (status, body) = self.authed(&mut live, "POST", &url, None).await?;
        if !(200..300).contains(&status) {
            return Err(refused(company_error(
                &live.conversation.domain,
                action,
                status,
                &body,
            )));
        }
        self.save(&live).await?;
        Ok(json!({
            "company": live.conversation.domain,
            "conversation_id": live.conversation.state.conversation_id,
            "status": body.get("status"),
            "responder": body.get("responder"),
        }))
    }
}

#[async_trait]
impl PoppyInvoker for PoppyClient {
    async fn invoke(&self, call: PoppyCall) -> Result<Value, String> {
        match self.run(call).await {
            Ok(answer) => Ok(answer),
            Err(PoppyError::Refused(message)) => Err(message),
            Err(PoppyError::Internal(message)) => {
                tracing::warn!(%message, "Poppy call failed");
                Err("Poppy is unavailable right now.".to_string())
            }
        }
    }
}

/// Pure: a refusal sentence, with the company's error code and description.
fn company_error(domain: &str, step: &str, status: u16, body: &Value) -> String {
    let code = body
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("HTTP {status}"));
    match body.get("error_description").and_then(Value::as_str) {
        Some(description) => format!("{domain} refused the {step}: {code} ({description})"),
        None => format!("{domain} refused the {step}: {code}"),
    }
}

/// Pure: take the events in `body`, advance the cursor, and return the
/// company's messages (the personal agent's own echoes left out).
fn absorb(state: &mut ConversationState, body: &Value) -> Vec<Value> {
    let events = body
        .get("events")
        .and_then(|events| events.get("events").or(Some(events)))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let cursor = body
        .get("cursor")
        .or_else(|| body.get("events").and_then(|events| events.get("cursor")))
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty());
    if let Some(cursor) = cursor {
        state.cursor = Some(cursor.to_string());
    }
    events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|event| event.get("message"))
        // `role` says whose side spoke (spec 7.4); `sender` only whether a
        // person or an agent did.
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("company"))
        .map(|message| {
            json!({
                "from": message.get("sender"),
                "text": message.get("text"),
                "data": message.get("data"),
            })
        })
        .collect()
}

fn answer(live: &Live, body: &Value, messages: Vec<Value>) -> Value {
    let status = body
        .get("status")
        .or_else(|| body.get("events").and_then(|events| events.get("status")));
    let responder = body.get("responder").or_else(|| {
        body.get("events")
            .and_then(|events| events.get("responder"))
    });
    json!({
        "company": live.conversation.domain,
        "conversation_id": live.conversation.state.conversation_id,
        "status": status,
        "responder": responder,
        "messages": messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absorbs_company_messages_and_the_cursor() {
        let mut state = ConversationState::default();
        let body = json!({
            "conversation_id": "cnv_1",
            "status": "idle",
            "events": {
                "events": [
                    { "id": "evt_1", "type": "message",
                      "message": { "id": "msg_a", "role": "user", "sender": "agent", "text": "Is it late?" } },
                    { "id": "evt_2", "type": "state", "state": "working" },
                    { "id": "evt_3", "type": "message",
                      "message": { "id": "msg_b", "role": "company", "sender": "agent", "text": "No." } }
                ],
                "cursor": "evt_3"
            }
        });
        let messages = absorb(&mut state, &body);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["text"], "No.");
        assert_eq!(state.cursor.as_deref(), Some("evt_3"));

        // A read answer carries the events at the top level.
        let read = json!({ "events": [], "cursor": "evt_9", "status": "idle" });
        assert!(absorb(&mut state, &read).is_empty());
        assert_eq!(state.cursor.as_deref(), Some("evt_9"));
    }

    #[test]
    fn errors_name_the_company_and_its_code() {
        assert_eq!(
            company_error(
                "shop.example",
                "message",
                409,
                &json!({ "error": "conversation_closed", "error_description": "Closed" })
            ),
            "shop.example refused the message: conversation_closed (Closed)"
        );
        assert_eq!(
            company_error("shop.example", "read", 500, &Value::Null),
            "shop.example refused the read: HTTP 500"
        );
    }

    #[tokio::test]
    async fn the_memory_store_keeps_one_identity_per_issuer() {
        let store = InMemoryPoppyStore::new();
        let first = store.account("https://a.example").await.unwrap();
        let again = store.account("https://a.example").await.unwrap();
        let other = store.account("https://b.example").await.unwrap();
        assert_eq!(first.user_id, again.user_id);
        assert_eq!(first.dpop.kid(), again.dpop.kid());
        assert_ne!(first.user_id, other.user_id);
        assert_ne!(first.dpop.kid(), other.dpop.kid());
    }
}
