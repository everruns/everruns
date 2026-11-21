//! The turn's person's own MCP servers (`user_mcp`) on the data plane
//! (data plane v2 parity, Phase B: Console Chat).
//!
//! The person's server list is theirs across every chat and agent of the
//! org, so it stays in the control plane (`data_plane/user_mcp.rs`). A
//! session reaches it two ways, both host calls that act as the turn's
//! person, whom the session's cell names, never the executor:
//!
//! - The manage tools' store (`UserMcpStoreExt`): each call goes over as
//!   [`USER_MCP_STORE`], bound to the turn the session has open
//!   ([`OPEN_TURN`]).
//! - The turn's servers: asked once per turn ([`USER_MCP_SERVERS`], for
//!   the open turn's input message), and they join the turn through the
//!   runtime's `TurnMcpServers` hook, under the agent's own.
//!
//! The same call brings the session's own MCP records (chat-only servers
//! and ARD MCP targets, kept in the session's storage on this cell), which
//! join every agent's turns, not only `user_mcp` ones, as v1 folds them
//! into the session's `mcpServers`.
//!
//! Decisions:
//! - Only a turn whose input has a recorded sender asks for the person's
//!   servers: a channel message, a schedule run or a subagent's task has no
//!   person. A turn asks at all only when it wants the person's servers or
//!   the session has a record (a local read), so a session with neither
//!   makes no call.
//! - The records are read by the control plane from this cell, never sent
//!   with the call: an executor names no server.
//! - The servers join the turn's resolution only, never the session
//!   record: a person's key travels as a literal header, and a record
//!   written back (a session override) must not keep it.
//! - A server that signs in as the person has its grant taken out as an
//!   agent's own per-turn grant is (`mcp::take_bindings`) and kept here by
//!   session; `mcp::AgentMcpAuth` asks its token per use. Only for a
//!   request to that server's name and address, so an agent's own server of
//!   the same name never gets the person's token.
//! - A lookup that fails leaves the turn without the person's servers, as
//!   on v1 (they add to a turn and never break it), and is not asked again
//!   in that turn.
//! - Each session's latest turn is kept; past [`MAX_SESSIONS`] the record
//!   is cleared and refilled as turns come.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use everruns_capabilities::capabilities::{
    USER_MCP_CAPABILITY_ID, UserMcpCallInvoker, install_user_mcp_store, user_mcp_use_enabled,
};
use everruns_contracts::host_calls::{OPEN_TURN, USER_MCP_SERVERS, USER_MCP_STORE};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::ScopedMcpServers;
use everruns_core::host::{HostBackends, TurnMcpServers};
use everruns_core::mcp::{
    UserMcpStoreCall, UserMcpStoreError, UserMcpStoreReply, UserMcpStoreResult,
};
use everruns_core::session::ExecutionSession;
use everruns_core::session_mcp_servers::SESSION_MCP_SERVER_KV_PREFIX;
use everruns_internal_protocol::commands::InternalCommandTransport;
use parking_lot::Mutex;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::hosting::Port;
use crate::lookups::OrgLookups;
use crate::mcp::PerTurnGrant;

/// How many sessions' turns are kept before the record is cleared.
pub const MAX_SESSIONS: usize = 4096;

/// One session's latest turn: its servers, and the grants of those that
/// sign in as the person (server name to its address and grant).
struct Turn {
    message: MessageId,
    servers: ScopedMcpServers,
    grants: BTreeMap<String, (String, PerTurnGrant)>,
}

static TURNS: LazyLock<Mutex<HashMap<SessionId, Turn>>> = LazyLock::new(Mutex::default);

/// Whether an agent with `capabilities` has `user_mcp`, and if so whether
/// its turns use the person's servers (`use`, default on).
pub fn user_mcp(capabilities: &[everruns_contracts::CapabilityRef]) -> Option<bool> {
    capabilities
        .iter()
        .find(|c| c.id() == USER_MCP_CAPABILITY_ID)
        .map(|c| user_mcp_use_enabled(c.config_value()))
}

/// The person's grant for `server` at `url` in `session`'s latest turn.
pub(crate) fn grant(session: Uuid, server: &str, url: Option<&str>) -> Option<PerTurnGrant> {
    let turns = TURNS.lock();
    let (address, grant) = turns
        .get(&SessionId::from_uuid(session))?
        .grants
        .get(server)?;
    (Some(address.as_str()) == url).then(|| grant.clone())
}

/// Give `session`'s activation the `user_mcp` store and its turns the
/// person's servers, when its agent (`definition`) has `user_mcp` and the
/// control plane is reached.
pub(crate) fn attach(
    mut backends: HostBackends,
    lookups: Option<OrgLookups>,
    port: Port,
    session: SessionId,
    definition: &Value,
) -> HostBackends {
    let Some(lookups) = lookups else {
        return backends;
    };
    let uses = crate::host::package(definition)
        .ok()
        .and_then(|package| user_mcp(&package.manifest().capabilities));
    backends.extensions.hooks.turn_mcp_servers = Some(Arc::new(PersonServers {
        lookups: lookups.clone(),
        port,
        session,
        person: uses == Some(true),
    }));
    let Some(_) = uses else {
        return backends;
    };
    let invoker: Arc<dyn UserMcpCallInvoker> = Arc::new(CellUserMcpStore {
        lookups: lookups.clone(),
        session,
    });
    let inner = backends.tool_context_extensions_factory.take();
    backends.tool_context_extensions_factory = Some(Arc::new(move |org, session| {
        let mut extensions = inner
            .as_ref()
            .map(|factory| factory(org, session))
            .unwrap_or_default();
        install_user_mcp_store(&mut extensions, invoker.clone(), None);
        extensions
    }));
    backends
}

/// The `user_mcp` store of one session, in the control plane.
struct CellUserMcpStore {
    lookups: OrgLookups,
    session: SessionId,
}

#[async_trait]
impl UserMcpCallInvoker for CellUserMcpStore {
    async fn invoke(
        &self,
        input_message: Option<Uuid>,
        call: UserMcpStoreCall,
    ) -> UserMcpStoreResult<UserMcpStoreReply> {
        let internal =
            |error: &dyn std::fmt::Display| UserMcpStoreError::Internal(error.to_string());
        let params = json!({
            "session_id": self.session.to_string(),
            "input_message_id": input_message.map_or_else(|| OPEN_TURN.to_owned(), |m| m.to_string()),
            "call": serde_json::to_value(&call).map_err(|e| internal(&e))?,
        });
        let answer = match self
            .lookups
            .execute_internal_command(USER_MCP_STORE, params)
            .await
        {
            Ok(Ok(answer)) => answer,
            Ok(Err(error)) => return Err(UserMcpStoreError::Internal(error.message)),
            Err(error) => return Err(internal(&error)),
        };
        if let Some(error) = answer.get("error") {
            return Err(serde_json::from_value(error.clone()).map_err(|e| internal(&e))?);
        }
        serde_json::from_value(answer["ok"].clone()).map_err(|e| internal(&e))
    }
}

/// The servers a turn of `session` adds: its person's (when its agent has
/// `user_mcp` with `use` on) and the session's own MCP records.
struct PersonServers {
    lookups: OrgLookups,
    port: Port,
    session: SessionId,
    person: bool,
}

/// What a turn asks the control plane for.
struct Ask {
    message: MessageId,
    /// The person's servers: their message is recorded.
    person: bool,
    /// The session has MCP records.
    records: bool,
}

impl PersonServers {
    /// The open turn, and what it asks for.
    async fn open_turn(&self) -> Option<Ask> {
        let person = self.person;
        self.port
            .read(self.session, move |s| {
                let message = s.authority.open.and(s.authority.open_input)?;
                Some(Ask {
                    message,
                    person: person && s.platform.senders.of(message).is_some(),
                    records: s
                        .authority
                        .values
                        .keys()
                        .any(|key| key.starts_with(SESSION_MCP_SERVER_KV_PREFIX)),
                })
            })
            .await
            .ok()
            .flatten()
    }

    async fn ask(&self, ask: &Ask) -> ScopedMcpServers {
        let mut params = json!({
            "session_id": self.session.to_string(),
            "session_servers": ask.records,
        });
        if ask.person {
            params["input_message_id"] = json!(ask.message.uuid().to_string());
        }
        let answer = match self
            .lookups
            .execute_internal_command(USER_MCP_SERVERS, params)
            .await
        {
            Ok(Ok(answer)) => answer,
            Ok(Err(error)) => {
                tracing::warn!(session = %self.session, message = %error.message, "no person's MCP servers for the turn");
                return ScopedMcpServers::new();
            }
            Err(error) => {
                tracing::warn!(session = %self.session, %error, "no person's MCP servers for the turn");
                return ScopedMcpServers::new();
            }
        };
        if let Some(error) = answer.get("error") {
            tracing::warn!(session = %self.session, %error, "no person's MCP servers for the turn");
            return ScopedMcpServers::new();
        }
        serde_json::from_value(answer["ok"].clone()).unwrap_or_else(|error| {
            tracing::warn!(session = %self.session, %error, "the person's MCP servers did not parse");
            ScopedMcpServers::new()
        })
    }
}

#[async_trait]
impl TurnMcpServers for PersonServers {
    async fn servers(&self, session: &ExecutionSession) -> ScopedMcpServers {
        if session.id != self.session {
            return ScopedMcpServers::new();
        }
        let Some(ask) = self.open_turn().await else {
            return ScopedMcpServers::new();
        };
        if !ask.person && !ask.records {
            TURNS.lock().remove(&self.session);
            return ScopedMcpServers::new();
        }
        let message = ask.message;
        if let Some(turn) = TURNS.lock().get(&self.session)
            && turn.message == message
        {
            return turn.servers.clone();
        }
        let mut servers = self.ask(&ask).await;
        let taken = crate::mcp::take_bindings(&mut servers);
        let grants = taken
            .per_turn
            .into_iter()
            .filter_map(|(name, grant)| {
                let url = servers.get(&name)?.url.clone();
                Some((name, (url, grant)))
            })
            .collect();
        let mut turns = TURNS.lock();
        if turns.len() >= MAX_SESSIONS && !turns.contains_key(&self.session) {
            turns.clear();
        }
        turns.insert(
            self.session,
            Turn {
                message,
                servers: servers.clone(),
                grants,
            },
        );
        servers
    }
}

#[cfg(test)]
#[path = "user_mcp_tests.rs"]
mod tests;
