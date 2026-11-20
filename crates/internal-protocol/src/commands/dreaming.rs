//! `DreamingStore` and dreaming's Memory calls over the `worker_dreaming_state`
//! command: a v1 worker's, and a data plane session's (host call
//! `dreaming_state`, data plane v2 parity step 10).
use super::{InternalCommandTransport, call, call_secret};
use async_trait::async_trait;
use everruns_contracts::{error::AgentLoopError, error::Result, typed_id::SessionId};
use everruns_core::dreaming::{DreamMemoryScope, DreamStateRow, DreamingStore};
use serde_json::json;
use std::sync::Arc;
/// Scoped transport factory; due() uses only the internal worker authority.
pub struct CommandDreamingStore {
    pub transport: Arc<dyn Fn(i64) -> Arc<dyn InternalCommandTransport> + Send + Sync>,
}
#[async_trait]
impl DreamingStore for CommandDreamingStore {
    async fn get(&self, org: i64, key: &str) -> Result<Option<DreamStateRow>> {
        call_secret(
            self.transport.as_ref()(org).as_ref(),
            "Read dreaming state",
            "worker_dreaming_state",
            json!({"operation":"get","key":key}),
        )
        .await
    }
    async fn compare_exchange(
        &self,
        org: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        session_id: Option<SessionId>,
        due_at: Option<i64>,
    ) -> Result<bool> {
        call(self.transport.as_ref()(org).as_ref(),"Commit dreaming state","worker_dreaming_state",json!({"operation":"cas","key":key,"expected":expected,"value":value,"session_id":session_id,"due_at":due_at})).await
    }
    async fn due(&self, now: i64, limit: u32) -> Result<Vec<(i64, SessionId)>> {
        call(
            self.transport.as_ref()(1).as_ref(),
            "Find dreaming work",
            "worker_dreaming_state",
            json!({"operation":"due","now":now,"limit":limit}),
        )
        .await
    }
}
/// One data plane session's dreaming state, over its org's transport (the
/// org is the transport's; the one a caller names is not used).
///
/// Decision: scoped to the session as an Engine session's store is
/// (`SessionDreaming` in the SQLite hosting): it commits no other session's
/// obligations and discovers none, so a turn's timer never runs another
/// session's dream. Discovery is the executor's sweep, by org
/// ([`due_in_org`]).
pub struct SessionCommandDreamingStore {
    transport: Arc<dyn InternalCommandTransport>,
    session: SessionId,
}

impl SessionCommandDreamingStore {
    pub fn new(transport: Arc<dyn InternalCommandTransport>, session: SessionId) -> Self {
        Self { transport, session }
    }
}

#[async_trait]
impl DreamingStore for SessionCommandDreamingStore {
    async fn get(&self, _org: i64, key: &str) -> Result<Option<DreamStateRow>> {
        call_secret(
            self.transport.as_ref(),
            "Read dreaming state",
            "worker_dreaming_state",
            json!({"operation":"get","key":key}),
        )
        .await
    }
    async fn compare_exchange(
        &self,
        _org: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        session_id: Option<SessionId>,
        due_at: Option<i64>,
    ) -> Result<bool> {
        if session_id.is_some_and(|session| session != self.session) {
            return Err(AgentLoopError::config("cross-session maintenance access"));
        }
        call(
            self.transport.as_ref(),
            "Commit dreaming state",
            "worker_dreaming_state",
            json!({"operation":"cas","key":key,"expected":expected,"value":value,"session_id":session_id,"due_at":due_at}),
        )
        .await
    }
    async fn due(&self, _now: i64, _limit: u32) -> Result<Vec<(i64, SessionId)>> {
        Ok(Vec::new())
    }
}

/// The org's sessions on the data plane whose dream is due at `now`.
pub async fn due_in_org(
    transport: &dyn InternalCommandTransport,
    now: i64,
    limit: u32,
) -> Result<Vec<SessionId>> {
    let due: Vec<(i64, SessionId)> = call(
        transport,
        "Find dreaming work",
        "worker_dreaming_state",
        json!({"operation":"due_org","now":now,"limit":limit}),
    )
    .await?;
    Ok(due.into_iter().map(|(_, session)| session).collect())
}

pub async fn memory_scope(
    transport: &dyn InternalCommandTransport,
    session: SessionId,
    mount: &str,
) -> Result<Option<DreamMemoryScope>> {
    call(
        transport,
        "Authorize dreaming Memory",
        "worker_dreaming_state",
        json!({"operation":"scope","session_id":session,"mount":mount}),
    )
    .await
}

pub async fn create_artifact(
    transport: &dyn InternalCommandTransport,
    session: SessionId,
    path: &str,
    content: &str,
) -> Result<bool> {
    call(
        transport,
        "Create dreaming artifact",
        "worker_dreaming_state",
        json!({"operation":"create_artifact","session_id":session,"key":path,"value":content}),
    )
    .await
}
