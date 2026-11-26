//! Host-private Monty records. Never exposed to platform tools or change history.
use crate::domains::common::*;
use crate::domains::sessions::SESSION_MANAGE;
use everruns_contracts::{
    runtime::durable_tools::DurableToolResultStore,
    typed_id::{MessageId, SessionId},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct WorkerCodeExecution {
    pub session_id: SessionId,
    pub namespace: String,
    pub call_id: String,
    pub operation: String,
    pub tool_name: Option<String>,
    pub fingerprint: Option<String>,
    pub value: Option<Value>,
    pub status: Option<String>,
    pub token: Option<Uuid>,
}

impl WorkerCodeExecution {
    fn validate(&self) -> Result<(), CommandError> {
        // THREAT[TM-TENANT-001]: callers cannot select another session's records
        // or collide with the outer tool result namespace.
        let prefix = format!("code:{}:", self.session_id);
        if self
            .namespace
            .strip_prefix(&prefix)
            .and_then(|v| v.parse::<MessageId>().ok())
            .is_none()
            || self.call_id.is_empty()
            || self.call_id.len() > 512
            || self.tool_name.as_ref().is_some_and(|v| v.len() > 256)
            || self.fingerprint.as_ref().is_some_and(|v| v.len() > 128)
            || self
                .value
                .as_ref()
                .is_some_and(|v| v.to_string().len() > 2 * 1024 * 1024)
        {
            return Err(CommandError::bad_request("Invalid private code record"));
        }
        Ok(())
    }
}

#[command(name = "worker_code_execution", category = "sessions",
    description = "Internal: private code continuation and effect journal.",
    method = "POST", path = "/internal/sessions/{session_id}/code-execution", policy = SESSION_MANAGE)]
impl Command for WorkerCodeExecution {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        self.validate()?;
        crate::domains::session_storage::queries::verify_session_ownership(
            &ctx.db,
            ctx.org_id(),
            self.session_id,
        )
        .await?;
        let store = crate::storage::PgDurableToolResultStore::new(ctx.db.pool().clone());
        let failure = |e: everruns_contracts::error::AgentLoopError| {
            CommandError::internal(anyhow::anyhow!(e))
        };
        match self.operation.as_str() {
            "get" => Ok(json!(
                store
                    .get_tool_call_status(&self.namespace, &self.call_id)
                    .await
                    .map_err(failure)?
            )),
            "claim" => Ok(json!(
                store
                    .try_claim_tool_call(
                        &self.namespace,
                        &self.call_id,
                        self.tool_name
                            .as_deref()
                            .ok_or_else(|| CommandError::bad_request("Tool name required"))?,
                        self.fingerprint
                            .as_deref()
                            .ok_or_else(|| CommandError::bad_request("Fingerprint required"))?
                    )
                    .await
                    .map_err(failure)?
            )),
            "settle" => {
                let token = self
                    .token
                    .filter(|t| !t.is_nil())
                    .ok_or_else(|| CommandError::bad_request("Claim token required"))?;
                if self.status.as_deref() != Some("settled") {
                    return Err(CommandError::bad_request("Invalid code record status"));
                }
                Ok(json!(
                    store
                        .settle_tool_call(
                            &self.namespace,
                            &self.call_id,
                            self.value
                                .ok_or_else(|| CommandError::bad_request("Result required"))?,
                            "settled",
                            token
                        )
                        .await
                        .map_err(failure)?
                ))
            }
            _ => Err(CommandError::bad_request("Unknown code journal operation")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateHarnessRow, CreateSessionRow, StorageBackend};
    use everruns_contracts::typed_id::PrincipalId;
    use everruns_core::{Caller, DEFAULT_ORG_ID};
    use std::sync::Arc;
    async fn session(db: &Arc<StorageBackend>, org_id: i64) -> SessionId {
        // A session needs its org's built-in base harness.
        if db
            .get_harness_by_name(org_id, "base")
            .await
            .unwrap()
            .is_none()
        {
            db.create_harness(
                org_id,
                CreateHarnessRow {
                    localizations: Default::default(),
                    name: "base".to_string(),
                    display_name: None,
                    icon: None,
                    description: None,
                    intro_markdown: None,
                    short_description: None,
                    starters: json!([]),
                    system_prompt: Some(String::new()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: json!([]),
                    mcp_servers: json!({}),
                    network_access: None,
                    is_built_in: true,
                    embedder_metadata: Default::default(),
                },
            )
            .await
            .unwrap();
        }
        db.create_session(CreateSessionRow {
            org_id,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("code journal".to_string()),
            ..Default::default()
        })
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn private_records_are_scoped_and_compare_exchange_protected() {
        let db = Arc::new(StorageBackend::test_database());
        let id = session(&db, DEFAULT_ORG_ID).await;
        let other = session(&db, DEFAULT_ORG_ID).await;
        let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None);
        let namespace = format!("code:{id}:{}", MessageId::from_uuid(Uuid::new_v4()));
        let mut params = json!({"operation":"claim", "session_id":id, "namespace":namespace,
            "call_id":"checkpoint", "tool_name":"python.checkpoint", "fingerprint":"abc"});
        let run = |p| async {
            dispatch("worker_code_execution", p, &ctx)
                .await
                .map(|s| serde_json::from_str::<Value>(&s).unwrap())
        };
        let first = run(params.clone()).await.unwrap();
        let token = first["Claimed"]["claim_token"].clone();
        assert!(token.is_string());
        assert!(
            run(params.clone())
                .await
                .unwrap()
                .get("AlreadyRunning")
                .is_some()
        );
        params["operation"] = json!("settle");
        params["status"] = json!("settled");
        params["value"] = json!({"private":"snapshot"});
        params["token"] = json!(Uuid::new_v4());
        assert_eq!(run(params.clone()).await.unwrap(), json!(false));
        params["token"] = token;
        assert_eq!(run(params.clone()).await.unwrap(), json!(true));
        assert_eq!(run(params.clone()).await.unwrap(), json!(false));
        params["operation"] = json!("get");
        assert_eq!(
            run(params.clone()).await.unwrap()["Settled"]["result_json"]["private"],
            "snapshot"
        );
        params["session_id"] = json!(other);
        assert!(run(params.clone()).await.is_err());
        params["session_id"] = json!(id);
        let person = Caller {
            is_internal: false,
            user_id: Some(Uuid::nil()),
            role: everruns_core::organization::OrgRole::Owner,
            ..Caller::internal(DEFAULT_ORG_ID)
        };
        let person_ctx = Ctx::minimal_for_test(person, db.clone(), None);
        let denied = dispatch("worker_code_execution", params.clone(), &person_ctx)
            .await
            .unwrap_err();
        assert!(matches!(denied.kind, CommandErrorKind::Forbidden(_)));
        let other_org = Ctx::minimal_for_test(Caller::internal(999999), db.clone(), None);
        assert!(
            dispatch("worker_code_execution", params.clone(), &other_org)
                .await
                .is_err()
        );
        params["operation"] = json!("settle");
        params["token"] = json!(Uuid::nil());
        assert!(run(params.clone()).await.is_err());
        params["operation"] = json!("get");
        params["namespace"] = json!("outer-tool-record");
        assert!(run(params).await.is_err());
        assert!(WorkerCodeExecution::meta().is_internal());
    }
}
