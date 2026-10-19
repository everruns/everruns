//! Internal maintenance persistence. This command is never a model/public command.
use crate::domains::common::*;
use crate::domains::sessions::SESSION_MANAGE;
use crate::kernel_imports::Policy;
use everruns_contracts::typed_id::SessionId;
use everruns_core::dreaming::DreamStateRow;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct WorkerDreamingState {
    pub operation: String,
    #[serde(default)]
    pub key: String,
    pub expected: Option<i64>,
    pub value: Option<String>,
    pub session_id: Option<String>,
    pub due_at: Option<i64>,
    pub now: Option<i64>,
    pub limit: Option<u32>,
    pub mount: Option<String>,
}
#[command(
    name = "worker_dreaming_state",
    category = "dreaming",
    description = "Internal: dreaming state and Memory authorization.",
    method = "POST",
    path = "/internal/dreaming/state"
)]
impl Command for WorkerDreamingState {
    type Output = serde_json::Value;
    fn policy() -> Option<&'static Policy> {
        // Worker identity is enforced by the internal path; session management
        // also participates in the shared permission-resolver contract.
        Some(&SESSION_MANAGE)
    }
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        // Global due discovery remains infrastructure; tenant effects and budget
        // reservations re-read adoption before every operation, including revocation.
        if self.operation != "due" && !ctx.feature_flags.dreaming {
            return Err(CommandError::forbidden(
                "Dreaming requires organization adoption",
            ));
        }
        // State can retain bounded evidence plus four serialized proposals.
        // Artifact paths use filesystem limits, independently of opaque state keys.
        let key_limit = if self.operation == "create_artifact" {
            4096
        } else {
            512
        };
        if self.key.len() > key_limit || self.value.as_ref().is_some_and(|v| v.len() > 1024 * 1024)
        {
            return Err(CommandError::bad_request("dreaming state exceeds limit"));
        }
        let org = ctx.org_id();
        let session = self
            .session_id
            .as_ref()
            .map(|s| s.parse::<SessionId>())
            .transpose()
            .map_err(|_| CommandError::bad_request("invalid session"))?;
        if let Some(id) = session {
            crate::domains::session_storage::queries::verify_session_ownership(&ctx.db, org, id)
                .await?;
        }
        let pool = ctx.db.pool();
        match self.operation.as_str() {
            "create_artifact" => {
                let id = session.ok_or_else(|| CommandError::bad_request("session required"))?;
                let content = self
                    .value
                    .ok_or_else(|| CommandError::bad_request("content required"))?;
                if content.len() > 64 * 1024 {
                    return Err(CommandError::bad_request("artifact exceeds limit"));
                }
                let service =
                    crate::domains::session_files::WorkspaceFileService::new(ctx.db.clone());
                let result =
                    everruns_core::session_files::SessionFileSystem::create_file_if_absent(
                        &service, id, &self.key, &content,
                    )
                    .await
                    .map_err(|error| CommandError::internal(error.into()))?;
                Ok(serde_json::json!(result))
            }
            "scope" => {
                let id = session.ok_or_else(|| CommandError::bad_request("session required"))?;
                let mount = self
                    .mount
                    .ok_or_else(|| CommandError::bad_request("mount required"))?;
                let service =
                    crate::domains::session_files::WorkspaceFileService::new(ctx.db.clone());
                let scope = everruns_core::session_files::SessionFileSystem::dreaming_scope(
                    &service, id, &mount,
                )
                .await
                .map_err(|error| CommandError::internal(error.into()))?;
                Ok(serde_json::json!(scope))
            }
            "get" => {
                let row=sqlx::query("SELECT revision,value_encrypted,session_id,due_at FROM dreaming_state WHERE org_id=$1 AND key=$2").bind(org).bind(&self.key).fetch_optional(pool).await.map_err(|error| CommandError::internal(error.into()))?;
                let result = if let Some(r) = row {
                    let enc = ctx
                        .encryption
                        .as_ref()
                        .ok_or_else(|| CommandError::bad_request("dreaming requires encryption"))?;
                    let value = enc
                        .decrypt_to_string(&r.get::<Vec<u8>, _>("value_encrypted"))
                        .map_err(CommandError::internal)?;
                    Some(DreamStateRow {
                        revision: r.get("revision"),
                        value,
                        session_id: r
                            .get::<Option<uuid::Uuid>, _>("session_id")
                            .map(SessionId::from_uuid),
                        due_at: r.get("due_at"),
                    })
                } else {
                    None
                };
                Ok(serde_json::json!(result))
            }
            "cas" => {
                let enc = ctx
                    .encryption
                    .as_ref()
                    .ok_or_else(|| CommandError::bad_request("dreaming requires encryption"))?;
                let value = self
                    .value
                    .ok_or_else(|| CommandError::bad_request("value required"))?;
                let encrypted = enc.encrypt_string(&value).map_err(CommandError::internal)?;
                let changed=if let Some(rev)=self.expected{
                    sqlx::query("UPDATE dreaming_state SET revision=revision+1,value_encrypted=$1,session_id=$2,due_at=$3 WHERE org_id=$4 AND key=$5 AND revision=$6").bind(encrypted).bind(session.map(|s|s.uuid())).bind(self.due_at).bind(org).bind(self.key).bind(rev).execute(pool).await
                }else{
                    sqlx::query("INSERT INTO dreaming_state(org_id,key,revision,value_encrypted,session_id,due_at) VALUES($1,$2,1,$3,$4,$5) ON CONFLICT DO NOTHING").bind(org).bind(self.key).bind(encrypted).bind(session.map(|s|s.uuid())).bind(self.due_at).execute(pool).await
                }.map_err(|error| CommandError::internal(error.into()))?.rows_affected()==1;
                Ok(serde_json::json!(changed))
            }
            "due" => {
                // Authenticated internal workers already have cross-org execution authority.
                let policy = crate::records::FeatureFlagPolicy::current();
                if !policy.deployment_flags().dreaming {
                    return Ok(serde_json::json!([]));
                }
                let default_enabled = policy.for_org(&std::collections::HashMap::new()).dreaming;
                // Filter before LIMIT so revoked tenants cannot starve adopted work.
                let rows=sqlx::query("SELECT d.org_id,d.session_id FROM dreaming_state d LEFT JOIN org_feature_flags f ON f.org_id=d.org_id AND f.flag_name='dreaming' WHERE d.due_at<=$1 AND d.session_id IS NOT NULL AND COALESCE(f.enabled,$3) ORDER BY d.due_at LIMIT $2").bind(self.now.unwrap_or_else(||chrono::Utc::now().timestamp())).bind(i64::from(self.limit.unwrap_or(32).min(100))).bind(default_enabled).fetch_all(pool).await.map_err(|error| CommandError::internal(error.into()))?;
                let ids: Vec<(i64, SessionId)> = rows
                    .iter()
                    .map(|r| (r.get("org_id"), SessionId::from_uuid(r.get("session_id"))))
                    .collect();
                Ok(serde_json::json!(ids))
            }
            _ => Err(CommandError::bad_request("invalid dreaming operation")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::{Caller, DEFAULT_ORG_ID};
    use crate::storage::{EncryptionService, StorageBackend};
    use serde_json::json;
    use std::sync::Arc;
    fn ctx(db: Arc<StorageBackend>, org: i64) -> Ctx {
        let encryption = Arc::new(
            EncryptionService::new("v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
        );
        Ctx::minimal_for_test(Caller::internal(org), db, Some(encryption))
            .with_feature_flags(crate::records::FeatureFlags::all_enabled())
    }
    async fn run(ctx: &Ctx, value: serde_json::Value) -> serde_json::Value {
        serde_json::from_str(&dispatch("worker_dreaming_state", value, ctx).await.unwrap()).unwrap()
    }
    struct DenyMaintenance;
    impl everruns_core::PermissionResolver for DenyMaintenance {
        fn has_permission(
            &self,
            _caller: &Caller,
            _permission: &everruns_core::Permission,
        ) -> bool {
            false
        }
        fn caller_permissions(&self, _caller: &Caller) -> Vec<everruns_core::Permission> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn maintenance_honors_permission_resolver_before_state_or_memory_effects() {
        let db = Arc::new(StorageBackend::test_database());
        let mut c = ctx(db.clone(), DEFAULT_ORG_ID);
        c.permission_resolver = Arc::new(DenyMaintenance);
        for operation in ["get", "cas", "scope", "create_artifact", "due"] {
            let error = dispatch(
                "worker_dreaming_state",
                json!({"operation": operation, "key": "budget:denied", "value": "{}"}),
                &c,
            )
            .await
            .unwrap_err();
            assert!(matches!(error.kind, CommandErrorKind::Forbidden(_)));
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dreaming_state")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn disabled_adoption_blocks_queued_work_before_budget_or_memory_effects() {
        let db = Arc::new(StorageBackend::test_database());
        let mut c = ctx(db, DEFAULT_ORG_ID);
        c.feature_flags.dreaming = false;
        for operation in ["get", "cas", "scope", "create_artifact"] {
            assert!(
                dispatch(
                    "worker_dreaming_state",
                    json!({"operation": operation, "key":"job:disabled"}),
                    &c
                )
                .await
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn maintenance_cas_is_atomic_encrypted_and_tenant_scoped() {
        let db = Arc::new(StorageBackend::test_database());
        let c = ctx(db.clone(), DEFAULT_ORG_ID);
        let p =
            json!({"operation":"cas","key":"budget:memory","value":"sensitive private evidence"});
        let (a, b) = tokio::join!(run(&c, p.clone()), run(&c, p));
        assert_ne!(a, b);
        let row = run(&c, json!({"operation":"get","key":"budget:memory"})).await;
        assert_eq!(row["value"], "sensitive private evidence");
        assert_eq!(
            run(
                &c,
                json!({"operation":"cas","key":"budget:memory","expected":0,"value":"stale"})
            )
            .await,
            json!(false)
        );
        let other = ctx(db.clone(), 999999);
        assert_eq!(
            run(&other, json!({"operation":"get","key":"budget:memory"})).await,
            json!(null)
        );
        let bytes: Vec<u8> = sqlx::query_scalar(
            "SELECT value_encrypted FROM dreaming_state WHERE org_id=$1 AND key='budget:memory'",
        )
        .bind(DEFAULT_ORG_ID)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert!(!bytes.windows(9).any(|w| w == b"sensitive"));
    }
    #[tokio::test]
    async fn memory_artifacts_do_not_overwrite_and_archival_revokes_writes() {
        use crate::domains::session_files::memory_mounts::{
            MemoryMount, MemoryMountRouter, ensure_shared_memory,
        };
        let db = Arc::new(StorageBackend::test_database());
        let memory = ensure_shared_memory(&db, DEFAULT_ORG_ID, "dreaming-atomic-test")
            .await
            .unwrap();
        let mount = MemoryMount {
            org_id: DEFAULT_ORG_ID,
            memory_id: memory.id,
            mount_path: "/memory/shared".into(),
            readonly: false,
        };
        let router = MemoryMountRouter::new(db.clone());
        let (a, b) = tokio::join!(
            router.create_file_if_absent(&mount, "/note.md", "first"),
            router.create_file_if_absent(&mount, "/note.md", "second")
        );
        assert_ne!(a.unwrap(), b.unwrap());
        let content = router.read_file(&mount, "/note.md").await.unwrap().unwrap();
        assert!(
            !router
                .create_file_if_absent(&mount, "/note.md", "replacement")
                .await
                .unwrap()
        );
        assert_eq!(
            router.read_file(&mount, "/note.md").await.unwrap().unwrap(),
            content
        );
        sqlx::query("UPDATE memories SET status='archived' WHERE id=$1")
            .bind(memory.id)
            .execute(db.pool())
            .await
            .unwrap();
        assert!(!router.writable_for_dreaming(&mount).await.unwrap());
        assert!(
            !router
                .create_file_if_absent(&mount, "/late.md", "late")
                .await
                .unwrap()
        );
        assert!(
            router
                .read_file(&mount, "/late.md")
                .await
                .unwrap()
                .is_none()
        );
    }
    #[tokio::test]
    async fn internal_command_is_not_available_to_public_callers() {
        let db = Arc::new(StorageBackend::test_database());
        let mut c = ctx(db, DEFAULT_ORG_ID);
        c.caller.is_internal = false;
        assert!(
            dispatch("worker_dreaming_state", json!({"operation":"due"}), &c)
                .await
                .is_err()
        );
    }
}
