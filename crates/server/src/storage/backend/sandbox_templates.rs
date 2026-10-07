//! Sandbox Template storage across hosted and in-memory backends.

use super::*;
use crate::records::ResolvedSandboxSpec;
use crate::records::{SandboxTemplate, SandboxTemplateRevision, SandboxTemplateSpec};
use crate::storage::{PgSandboxCheckpointStore, PrimarySandboxRecord};
use everruns_contracts::typed_id::SessionId;
use everruns_contracts::typed_id::{SandboxTemplateId, SandboxTemplateRevisionId};

type SandboxTemplateDbRow = (
    uuid::Uuid,
    String,
    String,
    Option<String>,
    bool,
    String,
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Utc>,
    uuid::Uuid,
    i32,
    serde_json::Value,
    chrono::DateTime<chrono::Utc>,
);

fn sandbox_template_from_row(row: SandboxTemplateDbRow) -> Result<SandboxTemplate> {
    Ok(SandboxTemplate {
        public_id: SandboxTemplateId::from_uuid(row.0),
        name: row.1,
        display_name: row.2,
        description: row.3,
        is_managed: row.4,
        status: row.5,
        created_at: row.6,
        updated_at: row.7,
        current_revision: SandboxTemplateRevision {
            public_id: SandboxTemplateRevisionId::from_uuid(row.8),
            sandbox_template_id: SandboxTemplateId::from_uuid(row.0),
            revision: row.9,
            spec: serde_json::from_value(row.10)?,
            created_at: row.11,
        },
    })
}

const SANDBOX_TEMPLATE_SELECT: &str = r#"
    SELECT e.id, e.name, e.display_name, e.description, e.is_managed, e.status,
           e.created_at, e.updated_at, r.id, r.revision, r.profile, r.created_at
      FROM execution_environments e
      JOIN execution_environment_revisions r ON r.id = e.current_revision_id
"#;

// THREAT[TM-TENANT-019]: Every reusable Sandbox Template lookup is scoped by the
// authenticated organization. Revision reads join back through the owning
// template rather than accepting a globally unique id as authorization.

impl StorageBackend {
    pub async fn create_sandbox_template(
        &self,
        org_id: i64,
        name: &str,
        display_name: &str,
        description: Option<&str>,
        spec: &SandboxTemplateSpec,
        is_managed: bool,
    ) -> Result<SandboxTemplate> {
        crate::domains::sandbox_templates::resolution::resolve_spec(spec)
            .map_err(anyhow::Error::msg)?;
        {
            let db = self.database();
            let id = SandboxTemplateId::new();
            let revision_id = SandboxTemplateRevisionId::new();
            let now = chrono::Utc::now();
            let mut tx = db.tx_pool().begin().await?;
            sqlx::query(
                    "INSERT INTO execution_environments (id, org_id, name, display_name, description, is_managed, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$7)",
                )
                .bind(id.uuid())
                .bind(org_id)
                .bind(name)
                .bind(display_name)
                .bind(description)
                .bind(is_managed)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO execution_environment_revisions (id, environment_id, revision, profile, created_at) VALUES ($1,$2,1,$3,$4)")
                    .bind(revision_id.uuid()).bind(id.uuid()).bind(serde_json::to_value(spec)?).bind(now)
                    .execute(&mut *tx).await?;
            sqlx::query("UPDATE execution_environments SET current_revision_id=$2 WHERE id=$1")
                .bind(id.uuid())
                .bind(revision_id.uuid())
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(SandboxTemplate {
                public_id: id,
                name: name.to_string(),
                display_name: display_name.to_string(),
                description: description.map(str::to_string),
                is_managed,
                status: "active".into(),
                current_revision: SandboxTemplateRevision {
                    public_id: revision_id,
                    sandbox_template_id: id,
                    revision: 1,
                    spec: spec.clone(),
                    created_at: now,
                },
                created_at: now,
                updated_at: now,
            })
        }
    }

    pub async fn list_sandbox_templates(
        &self,
        org_id: i64,
        include_archived: bool,
    ) -> Result<Vec<SandboxTemplate>> {
        {
            let db = self.database();
            let sql = format!(
                "{SANDBOX_TEMPLATE_SELECT} WHERE e.org_id=$1 {} ORDER BY e.is_managed DESC, e.display_name",
                if include_archived {
                    ""
                } else {
                    "AND e.status='active'"
                }
            );
            sqlx::query_as::<_, SandboxTemplateDbRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .fetch_all(db.tx_pool())
                .await?
                .into_iter()
                .map(sandbox_template_from_row)
                .collect()
        }
    }

    pub async fn get_sandbox_template(
        &self,
        org_id: i64,
        id: SandboxTemplateId,
    ) -> Result<Option<SandboxTemplate>> {
        {
            let db = self.database();
            let sql = format!("{SANDBOX_TEMPLATE_SELECT} WHERE e.org_id=$1 AND e.id=$2");
            sqlx::query_as::<_, SandboxTemplateDbRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id)
                .bind(id.uuid())
                .fetch_optional(db.tx_pool())
                .await?
                .map(sandbox_template_from_row)
                .transpose()
        }
    }

    pub async fn get_sandbox_template_revision(
        &self,
        org_id: i64,
        id: SandboxTemplateRevisionId,
    ) -> Result<Option<SandboxTemplateRevision>> {
        {
            let db = self.database();
            let row: Option<(uuid::Uuid, uuid::Uuid, i32, serde_json::Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                    "SELECT r.id,r.environment_id,r.revision,r.profile,r.created_at FROM execution_environment_revisions r JOIN execution_environments e ON e.id=r.environment_id WHERE e.org_id=$1 AND r.id=$2",
                ).bind(org_id).bind(id.uuid()).fetch_optional(db.tx_pool()).await?;
            row.map(
                |(revision_id, environment_id, revision, profile, created_at)| {
                    Ok(SandboxTemplateRevision {
                        public_id: SandboxTemplateRevisionId::from_uuid(revision_id),
                        sandbox_template_id: SandboxTemplateId::from_uuid(environment_id),
                        revision,
                        spec: serde_json::from_value(profile)?,
                        created_at,
                    })
                },
            )
            .transpose()
        }
    }

    pub async fn revise_sandbox_template(
        &self,
        org_id: i64,
        id: SandboxTemplateId,
        spec: &SandboxTemplateSpec,
        display_name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<Option<SandboxTemplate>> {
        crate::domains::sandbox_templates::resolution::resolve_spec(spec)
            .map_err(anyhow::Error::msg)?;
        {
            let db = self.database();
            let mut tx = db.tx_pool().begin().await?;
            let current: Option<(bool, String, i32)> = sqlx::query_as(
                    "SELECT e.is_managed,e.status,r.revision FROM execution_environments e JOIN execution_environment_revisions r ON r.id=e.current_revision_id WHERE e.org_id=$1 AND e.id=$2 FOR UPDATE OF e",
                ).bind(org_id).bind(id.uuid()).fetch_optional(&mut *tx).await?;
            let Some((managed, status, revision)) = current else {
                return Ok(None);
            };
            if managed {
                anyhow::bail!("managed Sandbox Templates cannot be revised");
            }
            if status != "active" {
                anyhow::bail!("archived Sandbox Templates cannot be revised");
            }
            let revision_id = SandboxTemplateRevisionId::new();
            sqlx::query("INSERT INTO execution_environment_revisions (id,environment_id,revision,profile) VALUES ($1,$2,$3,$4)")
                    .bind(revision_id.uuid()).bind(id.uuid()).bind(revision + 1).bind(serde_json::to_value(spec)?).execute(&mut *tx).await?;
            sqlx::query("UPDATE execution_environments SET current_revision_id=$3, display_name=COALESCE($4,display_name), description=CASE WHEN $5 THEN $6 ELSE description END, updated_at=now() WHERE org_id=$1 AND id=$2")
                    .bind(org_id).bind(id.uuid()).bind(revision_id.uuid()).bind(display_name)
                    .bind(description.is_some()).bind(description.flatten()).execute(&mut *tx).await?;
            tx.commit().await?;
            self.get_sandbox_template(org_id, id).await
        }
    }

    pub async fn archive_sandbox_template(
        &self,
        org_id: i64,
        id: SandboxTemplateId,
    ) -> Result<bool> {
        {
            let db = self.database();
            Ok(sqlx::query("UPDATE execution_environments SET status='archived',updated_at=now() WHERE org_id=$1 AND id=$2 AND status='active' AND NOT is_managed")
                .bind(org_id).bind(id.uuid()).execute(db.tx_pool()).await?.rows_affected() == 1)
        }
    }

    pub async fn ensure_managed_bashkit_sandbox_template(
        &self,
        org_id: i64,
    ) -> Result<SandboxTemplate> {
        if let Some(existing) = self
            .list_sandbox_templates(org_id, true)
            .await?
            .into_iter()
            .find(|item| item.is_managed && item.name == "bashkit-virtual-workspace")
        {
            return Ok(existing);
        }
        self.create_sandbox_template(
            org_id,
            "bashkit-virtual-workspace",
            "Bashkit Virtual Workspace",
            Some("Managed recoverable virtual filesystem and Bash interpreter."),
            &crate::domains::sandbox_templates::resolution::managed_bashkit_sandbox_spec(),
            true,
        )
        .await
    }

    /// Pin the resolved specification exactly once for the lifetime of a Session.
    pub async fn pin_primary_sandbox(
        &self,
        session_id: SessionId,
        binding_name: &str,
        spec: &ResolvedSandboxSpec,
    ) -> Result<PrimarySandboxRecord> {
        {
            let db = self.database();
            PgSandboxCheckpointStore::new(db.pool().clone())
                .pin_primary_sandbox(session_id, binding_name, spec)
                .await
                .map_err(Into::into)
        }
    }

    /// Load the Session-owned logical primary Sandbox, when present.
    pub async fn get_primary_sandbox(
        &self,
        session_id: SessionId,
    ) -> Result<Option<PrimarySandboxRecord>> {
        {
            let db = self.database();
            PgSandboxCheckpointStore::new(db.pool().clone())
                .get_primary_sandbox(session_id)
                .await
                .map_err(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sandbox_template_revisions_are_immutable_and_managed_is_idempotent() {
        let db = StorageBackend::test_database();
        let org_id = 7;
        let original =
            crate::domains::sandbox_templates::resolution::managed_bashkit_sandbox_spec();
        let created = db
            .create_sandbox_template(org_id, "build", "Build", None, &original, false)
            .await
            .unwrap();
        let original_revision = created.current_revision.public_id;

        let mut revised = original;
        revised.lifecycle.idle_after_seconds = 600;
        let current = db
            .revise_sandbox_template(
                org_id,
                created.public_id,
                &revised,
                Some("Build Updated"),
                None,
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(current.current_revision.revision, 2);
        assert_ne!(current.current_revision.public_id, original_revision);
        assert_eq!(
            db.get_sandbox_template_revision(org_id, original_revision)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );

        let first = db
            .ensure_managed_bashkit_sandbox_template(org_id)
            .await
            .unwrap();
        let second = db
            .ensure_managed_bashkit_sandbox_template(org_id)
            .await
            .unwrap();
        assert_eq!(first.public_id, second.public_id);
        assert!(first.is_managed);
    }
}
