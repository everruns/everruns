//! Engine session hosting: authority transport and epoch-bound ports only.
use crate::{Authority, AuthorityClient};
use async_trait::async_trait;
use everruns::hosting::{
    AdmissionReceipt, HostingStatus, SessionActivation, SessionBinding, SessionEpoch,
    SessionHosting,
};
use everruns_contracts::{
    error::Result,
    typed_id::{MessageId, SessionId},
};
use everruns_core::{
    RuntimeMessage,
    events::{EventContext, EventRequest, InputMessageData, ToolCompletedData},
    host::{
        HostBackends,
        session_actor::store::{ActorFence, LeasedActor, SessionActorStore},
    },
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
    async fn resolve_waits(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        target: everruns_contracts::typed_id::TurnId,
        approvals: Vec<everruns::hosting::ApprovalCommit>,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        let authority = self.client.session(session);
        let fingerprint = json!({"turn_id":target,"approvals":approvals,"results":results});
        let retained: Option<Value> = authority
            .call(
                "command",
                json!({"id":format!("wait_resolution:{command}")}),
            )
            .await?;
        if let Some(retained) = retained {
            return if retained["fingerprint"] == fingerprint {
                Ok(())
            } else {
                Err(crate::client::rejected(409))
            };
        }
        #[derive(Deserialize)]
        struct Snapshot {
            pending: Vec<everruns::ToolCall>,
            now_ms: i64,
            open: Option<Value>,
        }
        let snapshot: Snapshot = authority.call("status", json!({})).await?;
        if snapshot
            .open
            .as_ref()
            .and_then(|open| open["turn_id"].as_str())
            != Some(target.to_string().as_str())
        {
            return Err(crate::client::rejected(409));
        }
        let now = chrono::DateTime::from_timestamp_millis(snapshot.now_ms).ok_or_else(|| {
            everruns_contracts::error::AgentLoopError::store("invalid authority clock")
        })?;
        let prepared = if approvals.is_empty() {
            Vec::new()
        } else {
            everruns::hosting::prepare_approvals(&approvals, &snapshot.pending, target, now)?
        };
        if !results.is_empty() {
            everruns::hosting::validate_tool_resolutions(&results, &snapshot.pending, now)?;
        }
        if prepared.is_empty() && results.is_empty() {
            return Err(everruns_contracts::error::AgentLoopError::config(
                "empty wait resolution",
            ));
        }
        let expected: Vec<_> = snapshot
            .pending
            .into_iter()
            .filter(|call| results.iter().any(|result| result.tool_call_id == call.id))
            .collect();
        authority.call::<Value>("resolve_waits",json!({"command_id":command,"turn_id":target,"approvals":approvals,"prepared":prepared,"results":results,"expected_pending":expected})).await?;
        Ok(())
    }
    async fn submit_approvals(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        target: everruns_contracts::typed_id::TurnId,
        decisions: Vec<everruns_contracts::execution_api::ToolApprovalAnswer>,
    ) -> Result<everruns_contracts::execution_api::SubmitToolApprovalsResponse> {
        let authority = self.client.session(session);
        let fingerprint = json!({"turn_id":target,"decisions":decisions});
        let retained: Option<Value> = authority
            .call(
                "command",
                json!({"id":format!("approval_submission:{command}")}),
            )
            .await?;
        if let Some(retained) = retained {
            if retained["fingerprint"] != fingerprint {
                return Err(crate::client::rejected(409));
            }
            return serde_json::from_value(retained["value"].clone()).map_err(|_| {
                everruns_contracts::error::AgentLoopError::store(
                    "invalid approval submission receipt",
                )
            });
        }
        #[derive(Deserialize)]
        struct Snapshot {
            now_ms: i64,
            pending: Vec<everruns::ToolCall>,
            open: Option<Value>,
        }
        let snapshot: Snapshot = authority.call("status", json!({})).await?;
        if snapshot
            .open
            .as_ref()
            .and_then(|open| open["turn_id"].as_str())
            != Some(target.to_string().as_str())
        {
            return Err(crate::client::rejected(409));
        }
        let (approvals, response) =
            everruns::hosting::prepare_approval_submission(&decisions, &snapshot.pending, target)?;
        let now = chrono::DateTime::from_timestamp_millis(snapshot.now_ms).ok_or_else(|| {
            everruns_contracts::error::AgentLoopError::store("invalid authority clock")
        })?;
        let prepared =
            everruns::hosting::prepare_approvals(&approvals, &snapshot.pending, target, now)?;
        authority.call("approve",json!({"command_id":command,"approvals":approvals,"prepared":prepared,"submission":{"fingerprint":fingerprint,"response":response}})).await
    }
    async fn configure(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        patch: everruns::hosting::SessionConfiguration,
    ) -> Result<()> {
        patch.validate()?;
        self.client
            .session(session)
            .call::<Value>("configure", json!({"command_id":command,"patch":patch}))
            .await?;
        Ok(())
    }
    async fn seed_history(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        messages: Vec<RuntimeMessage>,
    ) -> Result<bool> {
        everruns::hosting::validate_history_seed(&messages)?;
        let events: Vec<_> = messages
            .into_iter()
            .map(|message| {
                if message.role == everruns_core::message::RuntimeMessageRole::Agent {
                    EventRequest::new(
                        session,
                        EventContext::empty(),
                        everruns_core::events::OutputMessageCompletedData::new(message),
                    )
                } else {
                    EventRequest::new(
                        session,
                        EventContext::empty(),
                        InputMessageData::new(message),
                    )
                }
            })
            .collect();
        self.client
            .session(session)
            .call("seed", json!({"command_id":command,"events":events}))
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
    async fn approve_many(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        approvals: Vec<everruns::hosting::ApprovalCommit>,
    ) -> Result<()> {
        let authority = self.client.session(session);
        let retained: Option<Value> = authority
            .call("command", json!({"id":format!("approval:{command}")}))
            .await?;
        if let Some(retained) = retained {
            if retained["fingerprint"] != json!(approvals) {
                return Err(crate::client::rejected(409));
            }
            return Ok(());
        }
        #[derive(Deserialize)]
        struct Snapshot {
            now_ms: i64,
            pending: Vec<everruns::ToolCall>,
            open: Option<Value>,
        }
        let snapshot: Snapshot = authority.call("status", json!({})).await?;
        let turn = serde_json::from_value(
            snapshot.open.ok_or_else(|| {
                everruns_contracts::error::AgentLoopError::config("session has no open turn")
            })?["turn_id"]
                .clone(),
        )
        .map_err(|_| everruns_contracts::error::AgentLoopError::store("invalid open turn"))?;
        let now = chrono::DateTime::from_timestamp_millis(snapshot.now_ms).ok_or_else(|| {
            everruns_contracts::error::AgentLoopError::store("invalid authority clock")
        })?;
        let prepared =
            everruns::hosting::prepare_approvals(&approvals, &snapshot.pending, turn, now)?;
        authority
            .call::<Value>(
                "approve",
                json!({"command_id":command,"approvals":approvals,"prepared":prepared}),
            )
            .await?;
        Ok(())
    }
    async fn create(&self, session: SessionId, binding: &SessionBinding) -> Result<()> {
        self.client
            .session(session)
            .call::<Value>("bind", json!({"binding":binding}))
            .await?;
        Ok(())
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
    async fn admit(
        &self,
        session: SessionId,
        message: RuntimeMessage,
        cancelled: bool,
    ) -> Result<AdmissionReceipt> {
        let event = EventRequest::new(
            session,
            EventContext::empty(),
            InputMessageData::new(message),
        );
        self.client
            .session(session)
            .call("input", json!({"event":event,"cancelled":cancelled}))
            .await
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
    async fn resolve(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        target: everruns_contracts::typed_id::TurnId,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        let authority = self.client.session(session);
        let fingerprint = json!({"turn_id":target,"results":results});
        let retained: Option<Value> = authority
            .call("command", json!({"id":format!("resolution:{command}")}))
            .await?;
        if let Some(retained) = retained {
            return if retained["fingerprint"] == fingerprint {
                Ok(())
            } else {
                Err(crate::client::rejected(409))
            };
        }
        #[derive(Deserialize)]
        struct Pending {
            pending: Vec<everruns::ToolCall>,
            now_ms: i64,
            open: Option<Value>,
        }
        let status: Pending = authority.call("status", json!({})).await?;
        if status
            .open
            .as_ref()
            .and_then(|open| open["turn_id"].as_str())
            != Some(target.to_string().as_str())
        {
            return Err(crate::client::rejected(409));
        }
        let now = chrono::DateTime::from_timestamp_millis(status.now_ms).ok_or_else(|| {
            everruns_contracts::error::AgentLoopError::store("invalid authority clock")
        })?;
        everruns::hosting::validate_tool_resolutions(&results, &status.pending, now)?;
        let expected: Vec<_> = status
            .pending
            .into_iter()
            .filter(|call| results.iter().any(|result| result.tool_call_id == call.id))
            .collect();
        authority
            .call::<Value>(
                "resolve",
                json!({"resolution_id":command,"turn_id":target,"results":results,"expected_pending":expected}),
            )
            .await?;
        Ok(())
    }
    async fn cancel(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        input: MessageId,
    ) -> Result<()> {
        self.client
            .session(session)
            .call::<Value>(
                "cancel",
                json!({"command_id":command,"input_message_id":input}),
            )
            .await?;
        Ok(())
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
