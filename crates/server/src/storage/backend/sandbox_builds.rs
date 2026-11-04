//! Tenant-scoped build queue and atomic application of a verified artifact.
use super::*;
use crate::domains::sandbox_templates::builds::record::SandboxBuildConflict;
use crate::domains::sandbox_templates::builds::record::{SandboxBuildStatus, SandboxTemplateBuild};
use crate::domains::sandbox_templates::record::SandboxTemplate;
use everruns_contracts::typed_id::{
    SandboxTemplateBuildId, SandboxTemplateId, SandboxTemplateRevisionId,
};

impl StorageBackend {
    /// THREAT[TM-TENANT-022]: A copied provider selector can resolve only this
    /// tenant's notes for the exact provider account and immutable artifact.
    pub async fn sandbox_build_provenance(
        &self,
        org: i64,
        target: &crate::domains::sandbox_templates::record::SandboxTargetSpec,
    ) -> Result<Option<crate::domains::sandbox_templates::builds::record::SandboxBuildProvenance>>
    {
        use crate::domains::sandbox_templates::builds::record::SandboxBuildProvenance;
        use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;
        if target.provider.as_deref() != Some("daytona")
            || target.credential.source != SessionSandboxCredentialSource::Organization
        {
            return Ok(None);
        }
        let (Some(snapshot), Some(account)) = (
            target.options.get("snapshot").and_then(|s| s.as_str()),
            target.credential.connection_id,
        ) else {
            return Ok(None);
        };
        let record:Option<serde_json::Value>=sqlx::query_scalar("SELECT record FROM sandbox_template_builds WHERE org_id=$1 AND status='applied' AND record->>'snapshot_id'=$2 AND record#>>'{source_spec,target,credential,connection_id}'=$3 ORDER BY created_at DESC LIMIT 1")
            .bind(org).bind(snapshot).bind(account.to_string()).fetch_optional(self.database().tx_pool()).await?;
        record
            .map(|record| {
                let build: SandboxTemplateBuild = serde_json::from_value(record)?;
                Ok(SandboxBuildProvenance {
                    build_id: build.id,
                    snapshot_id: snapshot.into(),
                    built_at: build.created_at,
                    software_notes: build.software_notes,
                    verification_command: build.request.verification_command,
                })
            })
            .transpose()
    }

    /// Host-only encrypted cleanup authority, deliberately outside the public
    /// JSON record. It expires with the resource ownership, not the audit row.
    pub async fn sandbox_build_credential(
        &self,
        org: i64,
        id: SandboxTemplateBuildId,
    ) -> Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar::<_, Option<Vec<u8>>>(
            "SELECT credential_snapshot_encrypted FROM sandbox_template_builds WHERE org_id=$1 AND id=$2",
        )
        .bind(org)
        .bind(id.uuid())
        .fetch_optional(self.database().tx_pool())
        .await?
        .flatten())
    }
    pub async fn refresh_sandbox_build_credential(
        &self,
        org: i64,
        id: SandboxTemplateBuildId,
        encrypted: &[u8],
    ) -> Result<()> {
        sqlx::query("UPDATE sandbox_template_builds SET credential_snapshot_encrypted=$3 WHERE org_id=$1 AND id=$2 AND credential_snapshot_encrypted IS NOT NULL")
            .bind(org).bind(id.uuid()).bind(encrypted).execute(self.database().tx_pool()).await?;
        Ok(())
    }

    pub async fn insert_sandbox_build(
        &self,
        org: i64,
        build: &SandboxTemplateBuild,
    ) -> Result<bool> {
        let connection = build
            .source_spec
            .target
            .credential
            .connection_id
            .ok_or_else(|| anyhow::anyhow!("Missing build account"))?;
        let result=sqlx::query("INSERT INTO sandbox_template_builds(id,org_id,sandbox_template_id,status,record,credential_snapshot_encrypted) SELECT $1,$2,$3,$4,$5,c.access_token_encrypted FROM virtual_user_connections c JOIN virtual_users v ON v.id=c.virtual_user_id WHERE c.id=$6 AND v.org_id=$2 AND c.owner_scope='organization' AND c.provider='daytona' AND c.access_token_encrypted IS NOT NULL ON CONFLICT DO NOTHING")
            .bind(build.id.uuid()).bind(org).bind(build.sandbox_template_id.uuid())
            .bind(build.status.as_str()).bind(serde_json::to_value(build)?).bind(connection).execute(self.database().tx_pool()).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn get_sandbox_build(
        &self,
        org: i64,
        id: SandboxTemplateBuildId,
    ) -> Result<Option<SandboxTemplateBuild>> {
        let row: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT record FROM sandbox_template_builds WHERE org_id=$1 AND id=$2",
        )
        .bind(org)
        .bind(id.uuid())
        .fetch_optional(self.database().tx_pool())
        .await?;
        row.map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    pub async fn list_sandbox_builds(
        &self,
        org: i64,
        template: SandboxTemplateId,
        offset: u32,
    ) -> Result<Vec<SandboxTemplateBuild>> {
        let rows: Vec<serde_json::Value>=sqlx::query_scalar("SELECT record FROM sandbox_template_builds WHERE org_id=$1 AND sandbox_template_id=$2 ORDER BY created_at DESC,id DESC LIMIT 20 OFFSET $3")
            .bind(org).bind(template.uuid()).bind(i64::from(offset)).fetch_all(self.database().tx_pool()).await?;
        rows.into_iter()
            .map(|v| serde_json::from_value(v).map_err(Into::into))
            .collect()
    }

    pub async fn save_sandbox_build(&self, org: i64, build: &SandboxTemplateBuild) -> Result<()> {
        // A cancellation accepted while cleanup awaited the provider must win
        // the final Ready save. Keep its artifact and authority queued for cleanup.
        sqlx::query(r#"UPDATE sandbox_template_builds SET
            status=CASE WHEN $3='ready' AND COALESCE((record->>'cancel_requested')::boolean,false) THEN 'cancelled' ELSE $3 END,
            cleanup_pending=$4 OR ($3='ready' AND COALESCE((record->>'cancel_requested')::boolean,false)),
            credential_snapshot_encrypted=CASE WHEN $4 OR $3 IN ('queued','running','ready') THEN credential_snapshot_encrypted ELSE NULL END,
            record=$5 || jsonb_build_object('cancel_requested',COALESCE((record->>'cancel_requested')::boolean,false))
                || CASE WHEN $3='ready' AND COALESCE((record->>'cancel_requested')::boolean,false)
                    THEN '{"status":"cancelled","step":"cancelled","cleanup_pending":true}'::jsonb ELSE '{}'::jsonb END,
            updated_at=now() WHERE org_id=$1 AND id=$2 AND (status IN ('queued','running') OR status=$3)"#)
            .bind(org).bind(build.id.uuid()).bind(build.status.as_str()).bind(build.cleanup_pending)
            .bind(serde_json::to_value(build)?).execute(self.database().tx_pool()).await?;
        Ok(())
    }

    pub async fn cancel_sandbox_build(&self, org: i64, id: SandboxTemplateBuildId) -> Result<bool> {
        Ok(sqlx::query("UPDATE sandbox_template_builds SET record=jsonb_set(record,'{cancel_requested}','true'),updated_at=now() WHERE org_id=$1 AND id=$2 AND status IN ('queued','running')")
            .bind(org).bind(id.uuid()).execute(self.database().tx_pool()).await?.rows_affected()==1)
    }

    pub async fn pending_sandbox_builds(&self) -> Result<Vec<(i64, uuid::Uuid)>> {
        // Retain applied provenance and billed unused artifacts; bound only
        // cleaned-up terminal history. Rotate retries by last update so a
        // revoked account cannot starve newer work in other organizations.
        sqlx::query("DELETE FROM sandbox_template_builds WHERE id IN (SELECT id FROM sandbox_template_builds WHERE status IN ('failed','cancelled','discarded') AND NOT cleanup_pending AND created_at < now()-interval '30 days' LIMIT 100)")
            .execute(self.database().tx_pool()).await?;
        Ok(sqlx::query_as("SELECT org_id,id FROM sandbox_template_builds WHERE status IN ('queued','running') OR cleanup_pending ORDER BY updated_at LIMIT 20")
            .fetch_all(self.database().tx_pool()).await?)
    }

    pub async fn discard_sandbox_build(
        &self,
        org: i64,
        id: SandboxTemplateBuildId,
    ) -> Result<bool> {
        let mut tx = self.database().tx_pool().begin().await?;
        lock_build(&mut tx, id).await?;
        let changed=sqlx::query("UPDATE sandbox_template_builds SET status='discarded',cleanup_pending=cleanup_pending OR status='ready',record=record || jsonb_build_object('status','discarded','cleanup_pending',cleanup_pending OR status='ready'),updated_at=now() WHERE org_id=$1 AND id=$2 AND status IN ('ready','failed','cancelled')")
            .bind(org).bind(id.uuid()).execute(&mut *tx).await?.rows_affected()==1;
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn apply_sandbox_build(
        &self,
        org: i64,
        id: SandboxTemplateBuildId,
    ) -> Result<Option<SandboxTemplate>> {
        let mut tx = self.database().tx_pool().begin().await?;
        lock_build(&mut tx, id).await?;
        let value: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT record FROM sandbox_template_builds WHERE org_id=$1 AND id=$2 FOR UPDATE",
        )
        .bind(org)
        .bind(id.uuid())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(value) = value else { return Ok(None) };
        let mut build: SandboxTemplateBuild = serde_json::from_value(value)?;
        if build.status == SandboxBuildStatus::Applied {
            tx.commit().await?;
            return self
                .get_sandbox_template(org, build.sandbox_template_id)
                .await;
        }
        if build.status != SandboxBuildStatus::Ready || build.cleanup_pending {
            anyhow::bail!(SandboxBuildConflict(
                "Only a ready build with completed cleanup can be applied"
            ));
        }
        let current:Option<(uuid::Uuid,bool,String,i32)>=sqlx::query_as("SELECT e.current_revision_id,e.is_managed,e.status,r.revision FROM execution_environments e JOIN execution_environment_revisions r ON r.id=e.current_revision_id WHERE e.org_id=$1 AND e.id=$2 FOR UPDATE OF e")
            .bind(org).bind(build.sandbox_template_id.uuid()).fetch_optional(&mut *tx).await?;
        let Some((source, managed, status, revision)) = current else {
            return Ok(None);
        };
        if source != build.request.source_revision_id.uuid() || managed || status != "active" {
            anyhow::bail!(SandboxBuildConflict(
                "The source template changed or is no longer editable; build from its current revision"
            ));
        }
        let mut spec = build.source_spec.clone();
        let options = spec
            .target
            .options
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid target options"))?;
        options.insert(
            "snapshot".into(),
            serde_json::json!(
                build
                    .snapshot_id
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Missing snapshot"))?
            ),
        );
        options.remove("size");
        crate::domains::sandbox_templates::resolution::resolve_spec(&spec)
            .map_err(anyhow::Error::msg)?;
        let next = SandboxTemplateRevisionId::new();
        sqlx::query("INSERT INTO execution_environment_revisions(id,environment_id,revision,profile) VALUES($1,$2,$3,$4)")
            .bind(next.uuid()).bind(build.sandbox_template_id.uuid()).bind(revision+1).bind(serde_json::to_value(&spec)?).execute(&mut *tx).await?;
        sqlx::query("UPDATE execution_environments SET current_revision_id=$3,updated_at=now() WHERE org_id=$1 AND id=$2")
            .bind(org).bind(build.sandbox_template_id.uuid()).bind(next.uuid()).execute(&mut *tx).await?;
        build.status = SandboxBuildStatus::Applied;
        build.applied_revision_id = Some(next);
        build.updated_at = chrono::Utc::now();
        sqlx::query("UPDATE sandbox_template_builds SET status='applied',credential_snapshot_encrypted=NULL,record=$3,updated_at=now() WHERE org_id=$1 AND id=$2")
            .bind(org).bind(id.uuid()).bind(serde_json::to_value(&build)?).execute(&mut *tx).await?;
        tx.commit().await?;
        self.get_sandbox_template(org, build.sandbox_template_id)
            .await
    }
}

/// Never let API requests occupy the pool for the duration of an install or
/// provider cleanup. Short transaction contention (including concurrent
/// idempotent apply) may settle; long build ownership returns a safe conflict.
async fn lock_build(tx: &mut sqlx::PgConnection, id: SandboxTemplateBuildId) -> Result<()> {
    for _ in 0..20 {
        let held: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,5479))")
                .bind(format!("sandbox-build-{}", id.uuid()))
                .fetch_one(&mut *tx)
                .await?;
        if held {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    anyhow::bail!(SandboxBuildConflict(
        "Build work or cleanup is still running; retry shortly"
    ));
}
