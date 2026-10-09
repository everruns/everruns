//! Durable managed-Environment persistence for remote workers.

use chrono::{DateTime, Utc};
use everruns_capabilities::sandbox_state::{SandboxStateError, SandboxStateStore};
use everruns_contracts::sandbox_checkpoint::{
    NewSandboxCheckpoint, SandboxCheckpointError, SandboxCheckpointStore, SandboxRef,
};
use everruns_contracts::session_sandbox::SessionSandboxState;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::worker_link::grpc_service::worker::support::internal_status;
use crate::worker_link::grpc_service::*;

fn payload(request: &proto::SandboxPersistenceRequest) -> Result<Value, Status> {
    serde_json::from_slice(&request.payload_json)
        .map_err(|error| Status::invalid_argument(format!("Invalid sandbox payload: {error}")))
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, Status> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Status::invalid_argument(format!("Missing sandbox field: {name}")))
}

fn uuid_field(value: &Value, name: &str) -> Result<Uuid, Status> {
    field(value, name)?
        .parse()
        .map_err(|_| Status::invalid_argument(format!("Invalid sandbox UUID: {name}")))
}

fn encode(value: Value) -> Result<proto::SandboxPersistenceResponse, Status> {
    Ok(proto::SandboxPersistenceResponse {
        payload_json: serde_json::to_vec(&value)
            .map_err(|error| internal_status("Failed to encode sandbox payload", error))?,
        error_kind: None,
        error_json: None,
    })
}

fn state_error(error: SandboxStateError) -> proto::SandboxPersistenceResponse {
    let (kind, details) = match &error {
        SandboxStateError::StaleGeneration {
            sandbox_id,
            current,
            carried,
        } => (
            "stale_generation",
            json!({"sandbox_id": sandbox_id, "current": current, "carried": carried}),
        ),
        SandboxStateError::WrongSandbox { current, carried } => (
            "wrong_sandbox",
            json!({"current": current, "carried": carried}),
        ),
        SandboxStateError::StateTooLarge { field, size, limit } => (
            "state_too_large",
            json!({"field": field, "size": size, "limit": limit}),
        ),
        SandboxStateError::Storage(_) => ("storage", json!({})),
    };
    proto::SandboxPersistenceResponse {
        payload_json: Vec::new(),
        error_kind: Some(kind.to_string()),
        error_json: serde_json::to_vec(&json!({
            "message": error.to_string(),
            "details": details,
        }))
        .ok(),
    }
}

fn checkpoint_error(error: SandboxCheckpointError) -> proto::SandboxPersistenceResponse {
    let (kind, details) = match &error {
        SandboxCheckpointError::StaleGeneration {
            sandbox_id,
            current,
            carried,
        } => (
            "stale_generation",
            json!({"sandbox_id": sandbox_id, "current": current, "carried": carried}),
        ),
        SandboxCheckpointError::Storage(_) => ("storage", json!({})),
    };
    proto::SandboxPersistenceResponse {
        payload_json: Vec::new(),
        error_kind: Some(kind.to_string()),
        error_json: serde_json::to_vec(&json!({
            "message": error.to_string(),
            "details": details,
        }))
        .ok(),
    }
}

fn sandbox_ref(value: &SandboxRef) -> Value {
    json!({"id": value.id, "generation": value.generation})
}

fn expected_sandbox_ref(value: &Value) -> Result<Option<SandboxRef>, Status> {
    value
        .get("expected")
        .filter(|value| !value.is_null())
        .map(|expected| {
            Ok::<SandboxRef, Status>(SandboxRef {
                id: uuid_field(expected, "id")?,
                generation: expected
                    .get("generation")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Status::invalid_argument("Missing sandbox generation"))?,
            })
        })
        .transpose()
}

fn state_value(state: SessionSandboxState) -> Value {
    let sandbox = state.sandbox.as_ref().map(sandbox_ref);
    json!({"state": state, "sandbox": sandbox})
}

impl WorkerServiceImpl {
    pub(crate) async fn handle_sandbox_persistence(
        &self,
        request: Request<proto::SandboxPersistenceRequest>,
    ) -> Result<Response<proto::SandboxPersistenceResponse>, Status> {
        let request = request.into_inner();
        let value = payload(&request)?;
        let store = crate::storage::PgSandboxCheckpointStore::new(self.db.pool().clone());

        let response =
            match request.operation.as_str() {
                "load_current_state" => {
                    let session_id = everruns_contracts::typed_id::SessionId::from_uuid(
                        uuid_field(&value, "session_id")?,
                    );
                    match store.load_current_state(session_id).await {
                        Ok(state) => encode(json!({"state": state.map(state_value)}))?,
                        Err(error) => state_error(error),
                    }
                }
                "load_state" => {
                    let session_id = everruns_contracts::typed_id::SessionId::from_uuid(
                        uuid_field(&value, "session_id")?,
                    );
                    match store
                        .load_state(session_id, field(&value, "provider")?)
                        .await
                    {
                        Ok(state) => encode(json!({"state": state.map(state_value)}))?,
                        Err(error) => state_error(error),
                    }
                }
                "save_state" => {
                    let session_id = everruns_contracts::typed_id::SessionId::from_uuid(
                        uuid_field(&value, "session_id")?,
                    );
                    let state: SessionSandboxState = serde_json::from_value(
                        value
                            .get("state")
                            .cloned()
                            .ok_or_else(|| Status::invalid_argument("Missing sandbox state"))?,
                    )
                    .map_err(|error| {
                        Status::invalid_argument(format!("Invalid sandbox state: {error}"))
                    })?;
                    let expected = expected_sandbox_ref(&value)?;
                    match store
                        .save_state(session_id, &state, expected.as_ref())
                        .await
                    {
                        Ok(reference) => encode(json!({"sandbox": sandbox_ref(&reference)}))?,
                        Err(error) => state_error(error),
                    }
                }
                "delete_state" => {
                    let session_id = everruns_contracts::typed_id::SessionId::from_uuid(
                        uuid_field(&value, "session_id")?,
                    );
                    let expected = expected_sandbox_ref(&value)?;
                    match store
                        .delete_state(session_id, field(&value, "provider")?, expected.as_ref())
                        .await
                    {
                        Ok(deleted) => encode(json!({"deleted": deleted}))?,
                        Err(error) => state_error(error),
                    }
                }
                "ensure_sandbox" => {
                    let session_id = everruns_contracts::typed_id::SessionId::from_uuid(
                        uuid_field(&value, "session_id")?,
                    );
                    match store
                        .ensure_sandbox(session_id, field(&value, "provider")?)
                        .await
                    {
                        Ok(reference) => encode(json!({"sandbox": sandbox_ref(&reference)}))?,
                        Err(error) => checkpoint_error(error),
                    }
                }
                "record_checkpoint" => {
                    let checkpoint: NewSandboxCheckpoint =
                        serde_json::from_value(value.get("checkpoint").cloned().ok_or_else(
                            || Status::invalid_argument("Missing sandbox checkpoint"),
                        )?)
                        .map_err(|error| {
                            Status::invalid_argument(format!("Invalid sandbox checkpoint: {error}"))
                        })?;
                    match store.record_checkpoint(checkpoint).await {
                        Ok(checkpoint) => encode(json!({"checkpoint": checkpoint}))?,
                        Err(error) => checkpoint_error(error),
                    }
                }
                "attach_checkpoint" => match store
                    .attach_checkpoint(
                        uuid_field(&value, "sandbox_id")?,
                        uuid_field(&value, "checkpoint_id")?,
                        value
                            .get("generation")
                            .and_then(Value::as_i64)
                            .ok_or_else(|| {
                                Status::invalid_argument("Missing sandbox generation")
                            })?,
                    )
                    .await
                {
                    Ok(()) => encode(json!({}))?,
                    Err(error) => checkpoint_error(error),
                },
                "current_checkpoint" => match store
                    .current_checkpoint(uuid_field(&value, "sandbox_id")?)
                    .await
                {
                    Ok(checkpoint) => encode(json!({"checkpoint": checkpoint}))?,
                    Err(error) => checkpoint_error(error),
                },
                "rollback_current_checkpoint" => match store
                    .rollback_current_checkpoint(
                        uuid_field(&value, "sandbox_id")?,
                        uuid_field(&value, "checkpoint_id")?,
                        value
                            .get("generation")
                            .and_then(Value::as_i64)
                            .ok_or_else(|| {
                                Status::invalid_argument("Missing sandbox generation")
                            })?,
                    )
                    .await
                {
                    Ok(checkpoint) => encode(json!({"checkpoint": checkpoint}))?,
                    Err(error) => checkpoint_error(error),
                },
                "collect_unattached_checkpoints" => {
                    let before: DateTime<Utc> = field(&value, "before")?
                        .parse()
                        .map_err(|_| Status::invalid_argument("Invalid checkpoint cutoff"))?;
                    match store
                        .collect_unattached_checkpoints(
                            uuid_field(&value, "sandbox_id")?,
                            before,
                            value.get("limit").and_then(Value::as_i64).ok_or_else(|| {
                                Status::invalid_argument("Missing checkpoint limit")
                            })?,
                        )
                        .await
                    {
                        Ok(revisions) => encode(json!({"revisions": revisions}))?,
                        Err(error) => checkpoint_error(error),
                    }
                }
                _ => {
                    return Err(Status::invalid_argument(
                        "Unknown sandbox persistence operation",
                    ));
                }
            };

        Ok(Response::new(response))
    }
}
