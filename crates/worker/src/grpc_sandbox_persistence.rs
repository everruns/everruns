//! gRPC adapter for first-class managed-Environment state and checkpoints.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_internal_protocol::proto;
use everruns_platform::sandbox_checkpoint::{
    NewSandboxCheckpoint, SandboxCheckpoint, SandboxCheckpointError, SandboxCheckpointStore,
    SandboxRef,
};
use everruns_platform::sandbox_state::{SandboxStateError, SandboxStateStore};
use everruns_platform::session_sandbox::SessionSandboxState;
use everruns_contracts::typed_id::SessionId;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::grpc_adapters::GrpcClient;

#[derive(Clone)]
pub(crate) struct GrpcSandboxPersistenceStore {
    client: GrpcClient,
}

impl GrpcSandboxPersistenceStore {
    pub(crate) fn new(client: GrpcClient) -> Self {
        Self { client }
    }

    async fn call(&self, operation: &str, payload: Value) -> Result<Value, RemoteError> {
        let request = proto::SandboxPersistenceRequest {
            operation: operation.to_string(),
            payload_json: serde_json::to_vec(&payload)
                .map_err(|error| RemoteError::storage(error.to_string()))?,
        };
        let response = self
            .client
            .inner
            .lock()
            .await
            .sandbox_persistence(request)
            .await
            .map_err(|error| RemoteError::storage(error.to_string()))?
            .into_inner();
        if let Some(kind) = response.error_kind {
            let error = response
                .error_json
                .as_deref()
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
                .unwrap_or_else(|| json!({"message": "Sandbox persistence failed"}));
            return Err(RemoteError { kind, value: error });
        }
        serde_json::from_slice(&response.payload_json)
            .map_err(|error| RemoteError::storage(error.to_string()))
    }
}

#[derive(Debug)]
struct RemoteError {
    kind: String,
    value: Value,
}

impl RemoteError {
    fn storage(message: impl Into<String>) -> Self {
        Self {
            kind: "storage".to_string(),
            value: json!({"message": message.into()}),
        }
    }

    fn message(&self) -> String {
        self.value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Sandbox persistence failed")
            .to_string()
    }

    fn detail_uuid(&self, name: &str) -> Option<Uuid> {
        self.value.get("details")?.get(name)?.as_str()?.parse().ok()
    }

    fn detail_i64(&self, name: &str) -> Option<i64> {
        self.value.get("details")?.get(name)?.as_i64()
    }

    fn detail_usize(&self, name: &str) -> Option<usize> {
        self.value
            .get("details")?
            .get(name)?
            .as_u64()?
            .try_into()
            .ok()
    }

    fn into_state(self) -> SandboxStateError {
        match self.kind.as_str() {
            "stale_generation" => SandboxStateError::StaleGeneration {
                sandbox_id: self.detail_uuid("sandbox_id").unwrap_or_default(),
                current: self.detail_i64("current").unwrap_or_default(),
                carried: self.detail_i64("carried").unwrap_or_default(),
            },
            "wrong_sandbox" => SandboxStateError::WrongSandbox {
                current: self.detail_uuid("current").unwrap_or_default(),
                carried: self.detail_uuid("carried").unwrap_or_default(),
            },
            "state_too_large" => SandboxStateError::StateTooLarge {
                field: match self
                    .value
                    .get("details")
                    .and_then(|details| details.get("field"))
                    .and_then(Value::as_str)
                {
                    Some("provider_state") => "provider_state",
                    Some("metadata") => "metadata",
                    _ => "unknown",
                },
                size: self.detail_usize("size").unwrap_or_default(),
                limit: self.detail_usize("limit").unwrap_or_default(),
            },
            _ => SandboxStateError::Storage(self.message()),
        }
    }

    fn into_checkpoint(self) -> SandboxCheckpointError {
        match self.kind.as_str() {
            "stale_generation" => SandboxCheckpointError::StaleGeneration {
                sandbox_id: self.detail_uuid("sandbox_id").unwrap_or_default(),
                current: self.detail_i64("current").unwrap_or_default(),
                carried: self.detail_i64("carried").unwrap_or_default(),
            },
            _ => SandboxCheckpointError::Storage(self.message()),
        }
    }
}

fn sandbox_ref(value: &Value) -> Result<SandboxRef, String> {
    Ok(SandboxRef {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Missing sandbox id".to_string())?
            .parse()
            .map_err(|_| "Invalid sandbox id".to_string())?,
        generation: value
            .get("generation")
            .and_then(Value::as_i64)
            .ok_or_else(|| "Missing sandbox generation".to_string())?,
    })
}

fn decode_state(value: Value) -> Result<SessionSandboxState, String> {
    let mut state: SessionSandboxState = serde_json::from_value(
        value
            .get("state")
            .cloned()
            .ok_or_else(|| "Missing sandbox state".to_string())?,
    )
    .map_err(|error| error.to_string())?;
    state.sandbox = value
        .get("sandbox")
        .filter(|value| !value.is_null())
        .map(sandbox_ref)
        .transpose()?;
    Ok(state)
}

fn expected_value(expected: Option<&SandboxRef>) -> Value {
    expected
        .map(|reference| json!({"id": reference.id, "generation": reference.generation}))
        .unwrap_or(Value::Null)
}

#[async_trait]
impl SandboxStateStore for GrpcSandboxPersistenceStore {
    async fn load_current_state(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
        let value = self
            .call(
                "load_current_state",
                json!({"session_id": session_id.uuid()}),
            )
            .await
            .map_err(RemoteError::into_state)?;
        value
            .get("state")
            .filter(|value| !value.is_null())
            .cloned()
            .map(decode_state)
            .transpose()
            .map_err(SandboxStateError::Storage)
    }

    async fn load_state(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
        let value = self
            .call(
                "load_state",
                json!({"session_id": session_id.uuid(), "provider": provider}),
            )
            .await
            .map_err(RemoteError::into_state)?;
        value
            .get("state")
            .filter(|value| !value.is_null())
            .cloned()
            .map(decode_state)
            .transpose()
            .map_err(SandboxStateError::Storage)
    }

    async fn save_state(
        &self,
        session_id: SessionId,
        state: &SessionSandboxState,
        expected: Option<&SandboxRef>,
    ) -> Result<SandboxRef, SandboxStateError> {
        let value = self
            .call(
                "save_state",
                json!({
                    "session_id": session_id.uuid(),
                    "state": state,
                    "expected": expected_value(expected),
                }),
            )
            .await
            .map_err(RemoteError::into_state)?;
        sandbox_ref(
            value
                .get("sandbox")
                .ok_or_else(|| SandboxStateError::Storage("Missing sandbox reference".into()))?,
        )
        .map_err(SandboxStateError::Storage)
    }

    async fn delete_state(
        &self,
        session_id: SessionId,
        provider: &str,
        expected: Option<&SandboxRef>,
    ) -> Result<bool, SandboxStateError> {
        let value = self
            .call(
                "delete_state",
                json!({
                    "session_id": session_id.uuid(),
                    "provider": provider,
                    "expected": expected_value(expected),
                }),
            )
            .await
            .map_err(RemoteError::into_state)?;
        value
            .get("deleted")
            .and_then(Value::as_bool)
            .ok_or_else(|| SandboxStateError::Storage("Missing delete result".into()))
    }
}

#[async_trait]
impl SandboxCheckpointStore for GrpcSandboxPersistenceStore {
    async fn ensure_sandbox(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<SandboxRef, SandboxCheckpointError> {
        let value = self
            .call(
                "ensure_sandbox",
                json!({"session_id": session_id.uuid(), "provider": provider}),
            )
            .await
            .map_err(RemoteError::into_checkpoint)?;
        sandbox_ref(
            value.get("sandbox").ok_or_else(|| {
                SandboxCheckpointError::Storage("Missing sandbox reference".into())
            })?,
        )
        .map_err(SandboxCheckpointError::Storage)
    }

    async fn record_checkpoint(
        &self,
        checkpoint: NewSandboxCheckpoint,
    ) -> Result<SandboxCheckpoint, SandboxCheckpointError> {
        let value = self
            .call("record_checkpoint", json!({"checkpoint": checkpoint}))
            .await
            .map_err(RemoteError::into_checkpoint)?;
        serde_json::from_value(
            value
                .get("checkpoint")
                .cloned()
                .ok_or_else(|| SandboxCheckpointError::Storage("Missing checkpoint".into()))?,
        )
        .map_err(|error| SandboxCheckpointError::Storage(error.to_string()))
    }

    async fn attach_checkpoint(
        &self,
        sandbox_id: Uuid,
        checkpoint_id: Uuid,
        generation: i64,
    ) -> Result<(), SandboxCheckpointError> {
        self.call(
            "attach_checkpoint",
            json!({
                "sandbox_id": sandbox_id,
                "checkpoint_id": checkpoint_id,
                "generation": generation,
            }),
        )
        .await
        .map_err(RemoteError::into_checkpoint)?;
        Ok(())
    }

    async fn current_checkpoint(
        &self,
        sandbox_id: Uuid,
    ) -> Result<Option<SandboxCheckpoint>, SandboxCheckpointError> {
        let value = self
            .call("current_checkpoint", json!({"sandbox_id": sandbox_id}))
            .await
            .map_err(RemoteError::into_checkpoint)?;
        value
            .get("checkpoint")
            .filter(|value| !value.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| SandboxCheckpointError::Storage(error.to_string()))
    }

    async fn rollback_current_checkpoint(
        &self,
        sandbox_id: Uuid,
        checkpoint_id: Uuid,
        generation: i64,
    ) -> Result<Option<SandboxCheckpoint>, SandboxCheckpointError> {
        let value = self
            .call(
                "rollback_current_checkpoint",
                json!({
                    "sandbox_id": sandbox_id,
                    "checkpoint_id": checkpoint_id,
                    "generation": generation,
                }),
            )
            .await
            .map_err(RemoteError::into_checkpoint)?;
        value
            .get("checkpoint")
            .filter(|value| !value.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| SandboxCheckpointError::Storage(error.to_string()))
    }

    async fn collect_unattached_checkpoints(
        &self,
        sandbox_id: Uuid,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<String>, SandboxCheckpointError> {
        let value = self
            .call(
                "collect_unattached_checkpoints",
                json!({
                    "sandbox_id": sandbox_id,
                    "before": before.to_rfc3339(),
                    "limit": limit,
                }),
            )
            .await
            .map_err(RemoteError::into_checkpoint)?;
        serde_json::from_value(
            value
                .get("revisions")
                .cloned()
                .ok_or_else(|| SandboxCheckpointError::Storage("Missing revisions".into()))?,
        )
        .map_err(|error| SandboxCheckpointError::Storage(error.to_string()))
    }
}
