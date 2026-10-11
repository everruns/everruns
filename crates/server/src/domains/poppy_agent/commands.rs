//! `worker_poppy_call`: the worker's one way into the Poppy client.
//!
//! Decision: one internal command carrying a [`PoppyCall`], run as the
//! worker's org after confirming the session belongs to it. The session's
//! owner is the person the agent speaks for, so their User ID and DPoP key at
//! the company are looked up by (org, owner, issuer); the session holds the
//! conversation. A company's refusal comes back as `BadRequest`, which the
//! worker hands the model as a tool error; anything else is internal.

use std::sync::Arc;

use chrono::{Duration, Utc};
use everruns_capabilities::capabilities::PoppyCall;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use super::protocol::{Company, PoppyAgent, SessionToken, company_host};
use crate::channels::a2a::pact_keys::ProviderKey;
use crate::domains::common::*;
use crate::domains::sessions::SESSION_MANAGE;
use crate::kernel_imports::contracts::typed_id::{OrgId, SessionId};
use crate::storage::poppy_agent::{PoppyAgentAccountRow, PoppyAgentConversationRow};

/// Seconds before expiry a Session Token is renewed rather than used.
const TOKEN_MARGIN_SECS: i64 = 30;

/// What a session remembers about its conversation with one company.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct ConversationState {
    name: Option<String>,
    token_endpoint: String,
    conversations: String,
    poppy_session_id: Option<String>,
    conversation_id: Option<String>,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerPoppyCall {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// What to do, as the `poppy_client` tools name it.
    #[schema(value_type = Object)]
    pub call: PoppyCall,
}

#[command(
    name = "worker_poppy_call",
    category = "sessions",
    description = "Internal: run one Poppy personal-agent call for a session.",
    method = "POST",
    path = "/internal/sessions/{session_id}/poppy",
    policy = SESSION_MANAGE
)]
impl Command for WorkerPoppyCall {
    type Output = Value;

    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let session_id =
            crate::domains::session_storage::queries::parse_owned_session_id(&self.session_id)?;
        let session = ctx
            .db
            .get_session(ctx.org_id(), session_id)
            .await
            .map_err(CommandError::internal)?
            .ok_or_else(|| CommandError::not_found("Session"))?;
        let egress = ctx
            .egress_service
            .clone()
            .unwrap_or_else(|| ctx.capability_service.egress_service());
        let egress = Arc::new(crate::kernel_imports::ScopedEgressService::new(
            egress,
            crate::kernel_imports::EgressScope {
                org_id: everruns_core::org_public_id_from_internal(ctx.org_id())
                    .parse::<OrgId>()
                    .ok(),
                session_id: Some(session_id),
            },
        ));
        let key = super::signing_key(&ctx.db, ctx.encryption.as_ref())
            .await
            .map_err(CommandError::internal)?;
        let agent = PoppyAgent::new(
            egress,
            Arc::new(key),
            super::client_id(&super::public_base_url()),
        );
        let client = Client {
            ctx,
            agent,
            session_id,
            owner: session.owner_principal_id.uuid(),
        };
        client.run(self.call).await
    }
}

/// A company's refusal or an unreachable company, for the model.
fn refused(message: String) -> CommandError {
    CommandError::bad_request(message)
}

struct Client<'a> {
    ctx: &'a Ctx,
    agent: PoppyAgent,
    session_id: SessionId,
    owner: Uuid,
}

/// A conversation ready to use: its state, account key and live token.
struct Live {
    row: PoppyAgentConversationRow,
    state: ConversationState,
    user_id: String,
    dpop: ProviderKey,
    token: String,
}

impl Client<'_> {
    async fn run(&self, call: PoppyCall) -> Result<Value, CommandError> {
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

    fn encrypt(&self, bytes: &[u8]) -> Result<(Vec<u8>, bool), CommandError> {
        match &self.ctx.encryption {
            Some(encryption) => Ok((
                encryption.encrypt(bytes).map_err(CommandError::internal)?,
                true,
            )),
            None => Ok((bytes.to_vec(), false)),
        }
    }

    fn decrypt(&self, bytes: &[u8], encrypted: bool) -> Result<Vec<u8>, CommandError> {
        match (encrypted, &self.ctx.encryption) {
            (false, _) => Ok(bytes.to_vec()),
            (true, Some(encryption)) => encryption.decrypt(bytes).map_err(CommandError::internal),
            (true, None) => Err(CommandError::internal(anyhow::anyhow!(
                "Poppy credential is encrypted but encryption is off"
            ))),
        }
    }

    /// The person's User ID and DPoP key at `issuer`, created on first use.
    async fn account(&self, issuer: &str) -> Result<(String, ProviderKey), CommandError> {
        let db = &self.ctx.db;
        let org = self.ctx.org_id();
        let row = match db
            .poppy_agent_account(org, self.owner, issuer)
            .await
            .map_err(CommandError::internal)?
        {
            Some(row) => row,
            None => {
                let key = ProviderKey::generate_row(self.ctx.encryption.as_ref())
                    .map_err(CommandError::internal)?;
                let row = PoppyAgentAccountRow {
                    user_id: format!("usr_{}", Uuid::new_v4().simple()),
                    dpop_jwk: key.public_jwk,
                    dpop_key: key.private_key,
                    encrypted: key.encrypted,
                };
                db.insert_poppy_agent_account(org, self.owner, issuer, &row)
                    .await
                    .map_err(CommandError::internal)?
            }
        };
        let key = ProviderKey::from_row(
            &crate::storage::pact_delegation::PactSigningKeyRow {
                kid: String::new(),
                private_key: row.dpop_key,
                encrypted: row.encrypted,
                public_jwk: row.dpop_jwk,
            },
            self.ctx.encryption.as_ref(),
        )
        .map_err(CommandError::internal)?;
        Ok((row.user_id, key))
    }

    async fn stored(
        &self,
        domain: &str,
    ) -> Result<Option<PoppyAgentConversationRow>, CommandError> {
        self.ctx
            .db
            .poppy_agent_conversation(self.session_id.uuid(), domain)
            .await
            .map_err(CommandError::internal)
    }

    async fn save(&self, live: &mut Live) -> Result<(), CommandError> {
        live.row.state = serde_json::to_value(&live.state)
            .map_err(|error| CommandError::internal(error.into()))?;
        self.ctx
            .db
            .upsert_poppy_agent_conversation(self.ctx.org_id(), &live.row)
            .await
            .map_err(CommandError::internal)
    }

    /// Start a Session (or renew the one held) and store its token.
    async fn refresh(&self, live: &mut Live) -> Result<(), CommandError> {
        let company = Company {
            domain: live.row.domain.clone(),
            name: live.state.name.clone(),
            issuer: live.row.issuer.clone(),
            token_endpoint: live.state.token_endpoint.clone(),
            revocation_endpoint: None,
            conversations: Some(live.state.conversations.clone()),
            document: Value::Null,
        };
        let renewed = match &live.state.poppy_session_id {
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
        let (sealed, encrypted) = self.encrypt(token.access_token.as_bytes())?;
        live.row.access_token = Some(sealed);
        live.row.token_sealed = encrypted;
        live.row.token_expires_at = Some(Utc::now() + Duration::seconds(token.expires_in.max(0)));
        live.state.poppy_session_id = Some(token.session_id);
        live.token = token.access_token;
        self.save(live).await
    }

    /// The conversation with `company` this session holds, ready to use:
    /// discovered and with a Session on first use (or when `fresh`).
    async fn live(&self, company: &str, fresh: bool) -> Result<Live, CommandError> {
        let domain = company_host(company).map_err(refused)?;
        let stored = if fresh {
            None
        } else {
            self.stored(&domain).await?
        };
        let (row, state) = match stored {
            Some(row) => {
                let state: ConversationState =
                    serde_json::from_value(row.state.clone()).unwrap_or_default();
                (row, state)
            }
            None => {
                let discovered = self.agent.discover(&domain).await.map_err(refused)?;
                let conversations = discovered.conversations.clone().ok_or_else(|| {
                    refused(format!("{domain} offers no conversations with its agent"))
                })?;
                let previous = self.stored(&domain).await?;
                let state = ConversationState {
                    name: discovered.name.clone(),
                    token_endpoint: discovered.token_endpoint.clone(),
                    conversations,
                    // A new conversation keeps the Session it can renew.
                    poppy_session_id: previous.as_ref().and_then(|row| {
                        serde_json::from_value::<ConversationState>(row.state.clone())
                            .ok()
                            .and_then(|state| state.poppy_session_id)
                            .filter(|_| row.issuer == discovered.issuer)
                    }),
                    conversation_id: None,
                    cursor: None,
                };
                let row = PoppyAgentConversationRow {
                    session_id: self.session_id.uuid(),
                    domain: domain.clone(),
                    issuer: discovered.issuer.clone(),
                    state: Value::Null,
                    access_token: previous.as_ref().and_then(|row| row.access_token.clone()),
                    token_sealed: previous.as_ref().is_some_and(|row| row.token_sealed),
                    token_expires_at: previous.as_ref().and_then(|row| row.token_expires_at),
                };
                (row, state)
            }
        };
        let (user_id, dpop) = self.account(&row.issuer).await?;
        let usable = row
            .token_expires_at
            .is_some_and(|at| at - Duration::seconds(TOKEN_MARGIN_SECS) > Utc::now());
        let token = match (&row.access_token, usable) {
            (Some(sealed), true) => {
                String::from_utf8(self.decrypt(sealed, row.token_sealed)?).unwrap_or_default()
            }
            _ => String::new(),
        };
        let mut live = Live {
            row,
            state,
            user_id,
            dpop,
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
    ) -> Result<(u16, Value), CommandError> {
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

    fn conversation_url(live: &Live) -> Result<String, CommandError> {
        match &live.state.conversation_id {
            Some(id) => Ok(format!("{}/{id}", live.state.conversations)),
            None => Err(refused(format!(
                "there is no conversation with {} yet; send a message first",
                live.row.domain
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
    ) -> Result<Value, CommandError> {
        let mut live = self.live(company, new_conversation).await?;
        // Without a cursor the answer reads from the start of the
        // conversation (spec 7.3), so a later message passes the last one.
        let mut query = format!("wait={wait_seconds}");
        if let Some(cursor) = live
            .state
            .conversation_id
            .as_ref()
            .and(live.state.cursor.as_ref())
        {
            query.push_str(&format!("&cursor={}", urlencoding::encode(cursor)));
        }
        let url = match &live.state.conversation_id {
            Some(id) => format!("{}/{id}/messages?{query}", live.state.conversations),
            None => format!("{}?{query}", live.state.conversations),
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
                &live.row.domain,
                "message",
                status,
                &body,
            )));
        }
        if let Some(id) = body.get("conversation_id").and_then(Value::as_str) {
            live.state.conversation_id = Some(id.to_string());
        }
        let messages = absorb(&mut live.state, &body);
        self.save(&mut live).await?;
        Ok(answer(&live, &body, messages))
    }

    async fn read(&self, company: &str, wait_seconds: u32) -> Result<Value, CommandError> {
        let mut live = self.live(company, false).await?;
        let base = Self::conversation_url(&live)?;
        let mut url = format!("{base}/events?wait={wait_seconds}");
        if let Some(cursor) = &live.state.cursor {
            url.push_str(&format!("&cursor={}", urlencoding::encode(cursor)));
        }
        let (status, body) = self.authed(&mut live, "GET", &url, None).await?;
        if status == 410 {
            // The cursor fell out of the company's window: start over.
            live.state.cursor = None;
            self.save(&mut live).await?;
            return Err(refused(
                "the company no longer has those events; read again to start from what it still has"
                    .into(),
            ));
        }
        if status != 200 {
            return Err(refused(company_error(
                &live.row.domain,
                "read",
                status,
                &body,
            )));
        }
        let messages = absorb(&mut live.state, &body);
        self.save(&mut live).await?;
        Ok(answer(&live, &body, messages))
    }

    async fn action(&self, company: &str, action: &str) -> Result<Value, CommandError> {
        let mut live = self.live(company, false).await?;
        let url = format!("{}/{action}", Self::conversation_url(&live)?);
        let (status, body) = self.authed(&mut live, "POST", &url, None).await?;
        if !(200..300).contains(&status) {
            return Err(refused(company_error(
                &live.row.domain,
                action,
                status,
                &body,
            )));
        }
        self.save(&mut live).await?;
        Ok(json!({
            "company": live.row.domain,
            "conversation_id": live.state.conversation_id,
            "status": body.get("status"),
            "responder": body.get("responder"),
        }))
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
        "company": live.row.domain,
        "conversation_id": live.state.conversation_id,
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
}
