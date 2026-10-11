//! `worker_poppy_call`: the worker's one way into the Poppy client.
//!
//! Decision: one internal command carrying a [`PoppyCall`], run as the
//! worker's org after confirming the session belongs to it. The session's
//! owner is the person the agent speaks for, so their User ID and DPoP key at
//! the company are looked up by (org, owner, issuer); the session holds the
//! conversation. The protocol flow is the library's `PoppyClient`; this file
//! is where its state lives. A company's refusal comes back as `BadRequest`,
//! which the worker hands the model as a tool error; anything else is
//! internal.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_capabilities::capabilities::poppy_client::protocol::PoppyAgent;
use everruns_capabilities::capabilities::{
    PoppyAccount, PoppyCall, PoppyClient, PoppyError, PoppyStore, StoredConversation,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domains::common::*;
use crate::domains::sessions::SESSION_MANAGE;
use crate::kernel_imports::contracts::typed_id::{OrgId, SessionId};
use crate::storage::poppy_agent::{PoppyAgentAccountRow, PoppyAgentConversationRow};
use crate::storage::{EncryptionService, StorageBackend};

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
        let store = DbPoppyStore {
            db: ctx.db.clone(),
            encryption: ctx.encryption.clone(),
            org_id: ctx.org_id(),
            owner: session.owner_principal_id.uuid(),
            session_id,
        };
        match PoppyClient::new(Arc::new(agent), Arc::new(store))
            .run(self.call)
            .await
        {
            Ok(answer) => Ok(answer),
            Err(PoppyError::Refused(message)) => Err(CommandError::bad_request(message)),
            Err(PoppyError::Internal(message)) => {
                Err(CommandError::internal(anyhow::anyhow!(message)))
            }
        }
    }
}

/// One session's Poppy state: the owner's accounts by (org, owner, issuer),
/// the session's conversations by company, Session Tokens sealed at rest.
struct DbPoppyStore {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    org_id: i64,
    owner: Uuid,
    session_id: SessionId,
}

#[async_trait]
impl PoppyStore for DbPoppyStore {
    async fn account(&self, issuer: &str) -> Result<PoppyAccount, String> {
        let encryption = self.encryption.as_ref();
        let stored = self
            .db
            .poppy_agent_account(self.org_id, self.owner, issuer)
            .await
            .map_err(|error| error.to_string())?;
        let row = match stored {
            Some(row) => row,
            None => {
                let account = PoppyAccount::generate()?;
                let key = super::key_row(&account.dpop, encryption).map_err(|e| e.to_string())?;
                let row = PoppyAgentAccountRow {
                    user_id: account.user_id,
                    dpop_jwk: key.public_jwk,
                    dpop_key: key.private_key,
                    encrypted: key.encrypted,
                };
                self.db
                    .insert_poppy_agent_account(self.org_id, self.owner, issuer, &row)
                    .await
                    .map_err(|error| error.to_string())?
            }
        };
        Ok(PoppyAccount {
            dpop: super::key_from_row(&row.dpop_key, row.encrypted, encryption)
                .map_err(|error| error.to_string())?,
            user_id: row.user_id,
        })
    }

    async fn conversation(&self, domain: &str) -> Result<Option<StoredConversation>, String> {
        let Some(row) = self
            .db
            .poppy_agent_conversation(self.session_id.uuid(), domain)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let token = match (row.access_token, row.token_expires_at) {
            (Some(sealed), Some(expires)) => {
                let bytes = match (row.token_sealed, &self.encryption) {
                    (false, _) => sealed,
                    (true, Some(encryption)) => {
                        encryption.decrypt(&sealed).map_err(|e| e.to_string())?
                    }
                    (true, None) => {
                        return Err("Poppy token is encrypted but encryption is off".into());
                    }
                };
                String::from_utf8(bytes).ok().map(|token| (token, expires))
            }
            _ => None,
        };
        Ok(Some(StoredConversation {
            domain: row.domain,
            issuer: row.issuer,
            state: serde_json::from_value(row.state).unwrap_or_default(),
            token,
        }))
    }

    async fn save_conversation(&self, conversation: &StoredConversation) -> Result<(), String> {
        let (access_token, token_sealed, token_expires_at) = match &conversation.token {
            Some((token, expires)) => match &self.encryption {
                Some(encryption) => (
                    Some(
                        encryption
                            .encrypt(token.as_bytes())
                            .map_err(|e| e.to_string())?,
                    ),
                    true,
                    Some(*expires),
                ),
                None => (Some(token.as_bytes().to_vec()), false, Some(*expires)),
            },
            None => (None, false, None),
        };
        let row = PoppyAgentConversationRow {
            session_id: self.session_id.uuid(),
            domain: conversation.domain.clone(),
            issuer: conversation.issuer.clone(),
            state: serde_json::to_value(&conversation.state).map_err(|e| e.to_string())?,
            access_token,
            token_sealed,
            token_expires_at,
        };
        self.db
            .upsert_poppy_agent_conversation(self.org_id, &row)
            .await
            .map_err(|error| error.to_string())
    }
}
