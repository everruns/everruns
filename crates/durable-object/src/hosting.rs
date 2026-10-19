//! Engine session hosting: authority transport and epoch-bound ports only.
use crate::{Authority, AuthorityClient};
use async_trait::async_trait;
use everruns::hosting::spi::{
    HostBackends,
    actor::{ActorFence, LeasedActor, SessionActorStore},
};
use everruns::hosting::spi::{
    error::Result,
    ids::{MessageId, SessionId},
};
use everruns::hosting::{
    AdmissionReceipt, HostingStatus, SessionActivation, SessionBinding, SessionEpoch,
    SessionHosting,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

/// Cloudflare and celld share this profile. Alarms retain the execution
/// obligation independently of the Engine/frontend process.
#[derive(Clone)]
pub struct ObjectHosting {
    client: AuthorityClient,
}

impl ObjectHosting {
    pub fn new(client: AuthorityClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SessionHosting for ObjectHosting {
    async fn command(
        &self,
        session: SessionId,
        command: everruns::hosting::spi::session_commands::SessionCommand,
    ) -> Result<Value> {
        self.client
            .session(session)
            .call("session_command", json!({"command":command}))
            .await
    }

    async fn query_snapshot(
        &self,
        session: SessionId,
    ) -> Result<everruns::hosting::SessionQuerySnapshot> {
        self.client
            .session(session)
            .call("snapshot", json!({}))
            .await
    }
    async fn event_sequence(&self, session: SessionId, event_id: &str) -> Result<Option<i32>> {
        self.client
            .session(session)
            .call("event_sequence", json!({"event_id":event_id}))
            .await
    }
    async fn status(&self, session: SessionId) -> Result<HostingStatus> {
        #[derive(Deserialize)]
        struct Status {
            holder: Option<String>,
            expires: i64,
            now_ms: i64,
            wake_at: Option<i64>,
            created_at: String,
            binding: SessionBinding,
            runnable: bool,
            suspended: bool,
            #[serde(default)]
            last_error: Option<String>,
            open: Option<Value>,
            pending: Vec<everruns::ToolCall>,
        }
        let status: Status = self
            .client
            .session(session)
            .call("status", json!({}))
            .await?;
        Ok(HostingStatus {
            executing: status.holder.is_some() && status.expires > status.now_ms,
            now_ms: status.now_ms,
            wake_at: status.wake_at,
            created_at: status.created_at,
            binding: status.binding,
            runnable: status.runnable,
            suspended: status.suspended,
            suspension_reason: if status.suspended {
                status.last_error
            } else {
                None
            },
            open_turn: status
                .open
                .and_then(|open| serde_json::from_value(open["turn_id"].clone()).ok()),
            waiting: status.pending,
        })
    }
    async fn receipt(
        &self,
        session: SessionId,
        message: MessageId,
    ) -> Result<Option<AdmissionReceipt>> {
        self.client
            .session(session)
            .call("receipt", json!({"message_id":message}))
            .await
    }
    fn reader(&self, session: SessionId) -> HostBackends {
        self.client.session(session).backends()
    }
    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>> {
        self.client
            .session(session)
            .call("framework", json!({"operation":"get", "key":key}))
            .await
    }
    async fn claim(
        &self,
        session: SessionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<SessionActivation>> {
        #[derive(Deserialize)]
        struct Claimed {
            binding: SessionBinding,
            actor: LeasedActor,
        }
        let claim: Option<Claimed> = self
            .client
            .session(session)
            .call("claim", json!({"holder":holder,"ttl_ms":ttl.as_millis()}))
            .await?;
        Ok(claim.map(|claim| SessionActivation {
            binding: claim.binding,
            actor: claim.actor.clone(),
            epoch: Arc::new(
                self.client
                    .session(session)
                    .fenced(holder, claim.actor.fence),
            ),
        }))
    }
}

#[async_trait]
impl SessionEpoch for Authority {
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        self.call::<Value>("schedule", json!({"wake_at":wake_at}))
            .await?;
        Ok(())
    }
    fn backends(&self) -> HostBackends {
        Authority::backends(self)
    }
    fn actor_store(&self) -> Arc<dyn SessionActorStore> {
        Arc::new(self.clone())
    }
    fn observe_fence(&self, fence: &ActorFence) {
        Authority::observe_fence(self, fence);
    }
    async fn record(&self, key: &str) -> Result<Option<Value>> {
        self.call("framework", json!({"operation":"get", "key":key}))
            .await
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        self.call("framework", json!({"operation":"list", "key":prefix}))
            .await
    }
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        self.call(
            "framework",
            json!({"operation":"set", "key":key,"value":value,"absent":absent}),
        )
        .await
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
        self.call::<Value>("suspend", json!({"reason":reason}))
            .await?;
        Ok(())
    }
}
