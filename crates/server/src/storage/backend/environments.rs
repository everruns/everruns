//! Logical execution-environment storage across hosted and in-memory backends.

use super::*;
use crate::records::ResolvedEnvironmentProfile;
use crate::records::{EnvironmentDefinition, EnvironmentProfile, EnvironmentRevision};
use crate::storage::{EnvironmentRecord, PgSandboxCheckpointStore};
use everruns_contracts::typed_id::SessionId;
use everruns_contracts::typed_id::{EnvironmentId, EnvironmentRevisionId};

type EnvironmentDbRow = (
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

fn environment_from_row(row: EnvironmentDbRow) -> Result<EnvironmentDefinition> {
    Ok(EnvironmentDefinition {
        public_id: EnvironmentId::from_uuid(row.0),
        name: row.1,
        display_name: row.2,
        description: row.3,
        is_managed: row.4,
        status: row.5,
        created_at: row.6,
        updated_at: row.7,
        current_revision: EnvironmentRevision {
            public_id: EnvironmentRevisionId::from_uuid(row.8),
            environment_id: EnvironmentId::from_uuid(row.0),
            revision: row.9,
            profile: serde_json::from_value(row.10)?,
            created_at: row.11,
        },
    })
}

const ENVIRONMENT_SELECT: &str = r#"
    SELECT e.id, e.name, e.display_name, e.description, e.is_managed, e.status,
           e.created_at, e.updated_at, r.id, r.revision, r.profile, r.created_at
      FROM execution_environments e
      JOIN execution_environment_revisions r ON r.id = e.current_revision_id
"#;

// THREAT[TM-TENANT-019]: Every reusable Environment lookup is scoped by the
// authenticated organization. Revision reads join back through the owning
// Environment rather than accepting a globally unique id as authorization.

impl StorageBackend {
    pub async fn create_environment_definition(
        &self,
        org_id: i64,
        name: &str,
        display_name: &str,
        description: Option<&str>,
        profile: &EnvironmentProfile,
        is_managed: bool,
    ) -> Result<EnvironmentDefinition> {
        crate::domains::environments::profiles::resolve_profile(profile)
            .map_err(anyhow::Error::msg)?;
        match self {
            Self::Postgres(db) => {
                let id = EnvironmentId::new();
                let revision_id = EnvironmentRevisionId::new();
                let now = chrono::Utc::now();
                let mut tx = db.pool().begin().await?;
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
                    .bind(revision_id.uuid()).bind(id.uuid()).bind(serde_json::to_value(profile)?).bind(now)
                    .execute(&mut *tx).await?;
                sqlx::query("UPDATE execution_environments SET current_revision_id=$2 WHERE id=$1")
                    .bind(id.uuid())
                    .bind(revision_id.uuid())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(EnvironmentDefinition {
                    public_id: id,
                    name: name.to_string(),
                    display_name: display_name.to_string(),
                    description: description.map(str::to_string),
                    is_managed,
                    status: "active".into(),
                    current_revision: EnvironmentRevision {
                        public_id: revision_id,
                        environment_id: id,
                        revision: 1,
                        profile: profile.clone(),
                        created_at: now,
                    },
                    created_at: now,
                    updated_at: now,
                })
            }
            Self::InMemory(db) => {
                if db
                    .environment_definitions
                    .read()
                    .values()
                    .any(|(owner, item)| {
                        *owner == org_id && item.status == "active" && item.name == name
                    })
                {
                    anyhow::bail!("environment name already exists");
                }
                let id = EnvironmentId::new();
                let revision_id = EnvironmentRevisionId::new();
                let now = chrono::Utc::now();
                let revision = EnvironmentRevision {
                    public_id: revision_id,
                    environment_id: id,
                    revision: 1,
                    profile: profile.clone(),
                    created_at: now,
                };
                let environment = EnvironmentDefinition {
                    public_id: id,
                    name: name.to_string(),
                    display_name: display_name.to_string(),
                    description: description.map(str::to_string),
                    is_managed,
                    status: "active".into(),
                    current_revision: revision.clone(),
                    created_at: now,
                    updated_at: now,
                };
                db.environment_revisions
                    .write()
                    .insert(revision_id, (org_id, revision));
                db.environment_definitions
                    .write()
                    .insert(id, (org_id, environment.clone()));
                Ok(environment)
            }
        }
    }

    pub async fn list_environment_definitions(
        &self,
        org_id: i64,
        include_archived: bool,
    ) -> Result<Vec<EnvironmentDefinition>> {
        match self {
            Self::Postgres(db) => {
                let sql = format!(
                    "{ENVIRONMENT_SELECT} WHERE e.org_id=$1 {} ORDER BY e.is_managed DESC, e.display_name",
                    if include_archived {
                        ""
                    } else {
                        "AND e.status='active'"
                    }
                );
                sqlx::query_as::<_, EnvironmentDbRow>(sqlx::AssertSqlSafe(sql.as_str()))
                    .bind(org_id)
                    .fetch_all(db.pool())
                    .await?
                    .into_iter()
                    .map(environment_from_row)
                    .collect()
            }
            Self::InMemory(db) => Ok(db
                .environment_definitions
                .read()
                .values()
                .filter(|(owner, item)| {
                    *owner == org_id && (include_archived || item.status == "active")
                })
                .map(|(_, item)| item.clone())
                .collect()),
        }
    }

    pub async fn get_environment_definition(
        &self,
        org_id: i64,
        id: EnvironmentId,
    ) -> Result<Option<EnvironmentDefinition>> {
        match self {
            Self::Postgres(db) => {
                let sql = format!("{ENVIRONMENT_SELECT} WHERE e.org_id=$1 AND e.id=$2");
                sqlx::query_as::<_, EnvironmentDbRow>(sqlx::AssertSqlSafe(sql.as_str()))
                    .bind(org_id)
                    .bind(id.uuid())
                    .fetch_optional(db.pool())
                    .await?
                    .map(environment_from_row)
                    .transpose()
            }
            Self::InMemory(db) => Ok(db
                .environment_definitions
                .read()
                .get(&id)
                .filter(|(owner, _)| *owner == org_id)
                .map(|(_, item)| item.clone())),
        }
    }

    pub async fn get_environment_revision(
        &self,
        org_id: i64,
        id: EnvironmentRevisionId,
    ) -> Result<Option<EnvironmentRevision>> {
        match self {
            Self::Postgres(db) => {
                let row: Option<(uuid::Uuid, uuid::Uuid, i32, serde_json::Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                    "SELECT r.id,r.environment_id,r.revision,r.profile,r.created_at FROM execution_environment_revisions r JOIN execution_environments e ON e.id=r.environment_id WHERE e.org_id=$1 AND r.id=$2",
                ).bind(org_id).bind(id.uuid()).fetch_optional(db.pool()).await?;
                row.map(
                    |(revision_id, environment_id, revision, profile, created_at)| {
                        Ok(EnvironmentRevision {
                            public_id: EnvironmentRevisionId::from_uuid(revision_id),
                            environment_id: EnvironmentId::from_uuid(environment_id),
                            revision,
                            profile: serde_json::from_value(profile)?,
                            created_at,
                        })
                    },
                )
                .transpose()
            }
            Self::InMemory(db) => Ok(db
                .environment_revisions
                .read()
                .get(&id)
                .filter(|(owner, _)| *owner == org_id)
                .map(|(_, revision)| revision.clone())),
        }
    }

    pub async fn revise_environment_definition(
        &self,
        org_id: i64,
        id: EnvironmentId,
        profile: &EnvironmentProfile,
        display_name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<Option<EnvironmentDefinition>> {
        crate::domains::environments::profiles::resolve_profile(profile)
            .map_err(anyhow::Error::msg)?;
        match self {
            Self::Postgres(db) => {
                let mut tx = db.pool().begin().await?;
                let current: Option<(bool, String, i32)> = sqlx::query_as(
                    "SELECT e.is_managed,e.status,r.revision FROM execution_environments e JOIN execution_environment_revisions r ON r.id=e.current_revision_id WHERE e.org_id=$1 AND e.id=$2 FOR UPDATE OF e",
                ).bind(org_id).bind(id.uuid()).fetch_optional(&mut *tx).await?;
                let Some((managed, status, revision)) = current else {
                    return Ok(None);
                };
                if managed {
                    anyhow::bail!("managed environments cannot be revised");
                }
                if status != "active" {
                    anyhow::bail!("archived environments cannot be revised");
                }
                let revision_id = EnvironmentRevisionId::new();
                sqlx::query("INSERT INTO execution_environment_revisions (id,environment_id,revision,profile) VALUES ($1,$2,$3,$4)")
                    .bind(revision_id.uuid()).bind(id.uuid()).bind(revision + 1).bind(serde_json::to_value(profile)?).execute(&mut *tx).await?;
                sqlx::query("UPDATE execution_environments SET current_revision_id=$3, display_name=COALESCE($4,display_name), description=CASE WHEN $5 THEN $6 ELSE description END, updated_at=now() WHERE org_id=$1 AND id=$2")
                    .bind(org_id).bind(id.uuid()).bind(revision_id.uuid()).bind(display_name)
                    .bind(description.is_some()).bind(description.flatten()).execute(&mut *tx).await?;
                tx.commit().await?;
                self.get_environment_definition(org_id, id).await
            }
            Self::InMemory(db) => {
                let mut definitions = db.environment_definitions.write();
                let Some((owner, item)) = definitions.get_mut(&id) else {
                    return Ok(None);
                };
                if *owner != org_id {
                    return Ok(None);
                }
                if item.is_managed {
                    anyhow::bail!("managed environments cannot be revised");
                }
                if item.status != "active" {
                    anyhow::bail!("archived environments cannot be revised");
                }
                let revision = EnvironmentRevision {
                    public_id: EnvironmentRevisionId::new(),
                    environment_id: id,
                    revision: item.current_revision.revision + 1,
                    profile: profile.clone(),
                    created_at: chrono::Utc::now(),
                };
                if let Some(value) = display_name {
                    item.display_name = value.to_string();
                }
                if let Some(value) = description {
                    item.description = value.map(str::to_string);
                }
                item.current_revision = revision.clone();
                item.updated_at = chrono::Utc::now();
                db.environment_revisions
                    .write()
                    .insert(revision.public_id, (org_id, revision));
                Ok(Some(item.clone()))
            }
        }
    }

    pub async fn archive_environment_definition(
        &self,
        org_id: i64,
        id: EnvironmentId,
    ) -> Result<bool> {
        match self {
            Self::Postgres(db) => Ok(sqlx::query("UPDATE execution_environments SET status='archived',updated_at=now() WHERE org_id=$1 AND id=$2 AND status='active' AND NOT is_managed")
                .bind(org_id).bind(id.uuid()).execute(db.pool()).await?.rows_affected() == 1),
            Self::InMemory(db) => {
                let mut definitions = db.environment_definitions.write();
                let Some((owner, item)) = definitions.get_mut(&id) else { return Ok(false); };
                if *owner != org_id || item.is_managed || item.status != "active" { return Ok(false); }
                item.status = "archived".into();
                item.updated_at = chrono::Utc::now();
                Ok(true)
            }
        }
    }

    pub async fn ensure_managed_bashkit_environment(
        &self,
        org_id: i64,
    ) -> Result<EnvironmentDefinition> {
        if let Some(existing) = self
            .list_environment_definitions(org_id, true)
            .await?
            .into_iter()
            .find(|item| item.is_managed && item.name == "bashkit-virtual-workspace")
        {
            return Ok(existing);
        }
        self.create_environment_definition(
            org_id,
            "bashkit-virtual-workspace",
            "Bashkit Virtual Workspace",
            Some("Managed recoverable virtual filesystem and Bash interpreter."),
            &crate::domains::environments::profiles::managed_bashkit_profile(),
            true,
        )
        .await
    }

    /// Pin the resolved profile exactly once for the lifetime of a Session.
    pub async fn pin_environment(
        &self,
        session_id: SessionId,
        profile_name: &str,
        profile: &ResolvedEnvironmentProfile,
    ) -> Result<EnvironmentRecord> {
        match self {
            Self::Postgres(db) => PgSandboxCheckpointStore::new(db.pool().clone())
                .pin_environment(session_id, profile_name, profile)
                .await
                .map_err(Into::into),
            Self::InMemory(db) => {
                if !db.sessions.read().contains_key(&session_id) {
                    anyhow::bail!("session not found while pinning environment");
                }
                let provider = profile
                    .target
                    .provider
                    .clone()
                    .unwrap_or_else(|| profile.target.kind.as_str().to_string());
                let mut environments = db.environments.write();
                if let Some(existing) = environments.get(&session_id) {
                    if existing.profile_name == profile_name
                        && existing.profile == *profile
                        && existing.provider == provider
                    {
                        return Ok(existing.clone());
                    }
                    anyhow::bail!("session environment is already pinned to a different profile");
                }
                let record = EnvironmentRecord {
                    id: uuid::Uuid::now_v7(),
                    session_id,
                    provider,
                    profile_name: profile_name.to_string(),
                    profile: profile.clone(),
                    environment_revision_id: profile.source_revision_id,
                    desired_state: "ready".to_string(),
                    observed_state: "absent".to_string(),
                    generation: 1,
                    current_checkpoint_id: None,
                    last_activity_at: None,
                };
                environments.insert(session_id, record.clone());
                Ok(record)
            }
        }
    }

    /// Load the Session-owned logical Environment, if the Session selected one.
    pub async fn get_environment(
        &self,
        session_id: SessionId,
    ) -> Result<Option<EnvironmentRecord>> {
        match self {
            Self::Postgres(db) => PgSandboxCheckpointStore::new(db.pool().clone())
                .get_environment(session_id)
                .await
                .map_err(Into::into),
            Self::InMemory(db) => Ok(db.environments.read().get(&session_id).cloned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reusable_environment_revisions_are_immutable_and_managed_is_idempotent() {
        let db = StorageBackend::in_memory();
        let org_id = 7;
        let original = crate::domains::environments::profiles::managed_bashkit_profile();
        let created = db
            .create_environment_definition(org_id, "build", "Build", None, &original, false)
            .await
            .unwrap();
        let original_revision = created.current_revision.public_id;

        let mut revised = original;
        revised.lifecycle.idle_after_seconds = 600;
        let current = db
            .revise_environment_definition(
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
            db.get_environment_revision(org_id, original_revision)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );

        let first = db.ensure_managed_bashkit_environment(org_id).await.unwrap();
        let second = db.ensure_managed_bashkit_environment(org_id).await.unwrap();
        assert_eq!(first.public_id, second.public_id);
        assert!(first.is_managed);
    }
}
