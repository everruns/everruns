//! Persistence for Application publication and explicitly shared resources.
use super::Database;
use crate::domains::applications::types::*;
use crate::domains::common::CommandError;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{AgentChannelId, ApplicationId, MemoryId, RoomId};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct ApplicationRow {
    pub id: Uuid,
    pub org_id: i64,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub config: serde_json::Value,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
impl Database {
    pub async fn application(
        &self,
        org: i64,
        id: ApplicationId,
    ) -> anyhow::Result<Option<ApplicationRow>> {
        Ok(
            sqlx::query_as("SELECT * FROM applications WHERE org_id=$1 AND id=$2")
                .bind(org)
                .bind(id.uuid())
                .fetch_optional(&self.pool)
                .await?,
        )
    }
    pub async fn application_by_slug(&self, slug: &str) -> anyhow::Result<Option<ApplicationRow>> {
        Ok(
            sqlx::query_as("SELECT * FROM applications WHERE slug=$1 AND status='published'")
                .bind(slug)
                .fetch_optional(&self.pool)
                .await?,
        )
    }
    pub async fn application_for_channel(
        &self,
        channel: Uuid,
    ) -> anyhow::Result<Option<ApplicationRow>> {
        Ok(sqlx::query_as("SELECT a.* FROM applications a JOIN application_resources r ON r.application_id=a.id WHERE r.channel_id=$1").bind(channel).fetch_optional(&self.pool).await?)
    }
    pub async fn application_resources(
        &self,
        id: Uuid,
    ) -> anyhow::Result<Vec<ApplicationResource>> {
        #[derive(FromRow)]
        struct Row {
            slug: String,
            name: String,
            channel_id: Option<String>,
            room_id: Option<Uuid>,
            memory_id: Option<String>,
            writable: bool,
        }
        let rows:Vec<Row>=sqlx::query_as("SELECT r.slug,r.name,c.public_id AS channel_id,r.room_id,m.public_id AS memory_id,r.writable FROM application_resources r LEFT JOIN agent_channels c ON c.id=r.channel_id LEFT JOIN memories m ON m.id=r.memory_id WHERE r.application_id=$1 ORDER BY r.position,r.slug").bind(id).fetch_all(&self.pool).await.map_err(anyhow::Error::from)?;
        rows.into_iter()
            .map(|r| {
                let (kind, resource_id) = if let Some(id) = r.channel_id {
                    (ApplicationResourceKind::Chat, id)
                } else if let Some(id) = r.room_id {
                    (
                        ApplicationResourceKind::Room,
                        RoomId::from_uuid(id).to_string(),
                    )
                } else {
                    (
                        ApplicationResourceKind::Memory,
                        r.memory_id.ok_or_else(|| {
                            anyhow::anyhow!("Application resource is unavailable")
                        })?,
                    )
                };
                Ok(ApplicationResource {
                    slug: r.slug,
                    name: r.name,
                    kind,
                    resource_id,
                    writable: r.writable,
                })
            })
            .collect()
    }
    pub async fn application_view(&self, row: ApplicationRow) -> anyhow::Result<Application> {
        Ok(Application {
            id: ApplicationId::from_uuid(row.id),
            public_url: public_url(&row.slug),
            slug: row.slug,
            name: row.name,
            description: row.description,
            status: row.status,
            config: serde_json::from_value(row.config)?,
            resources: self.application_resources(row.id).await?,
            version: row.version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
    pub async fn list_applications(&self, org: i64) -> anyhow::Result<Vec<Application>> {
        self.list_applications_including_archived(org, false).await
    }
    pub async fn list_applications_including_archived(
        &self,
        org: i64,
        include_archived: bool,
    ) -> anyhow::Result<Vec<Application>> {
        let rows=sqlx::query_as("SELECT * FROM applications WHERE org_id=$1 AND ($2 OR status<>'archived') ORDER BY created_at DESC LIMIT 200").bind(org).bind(include_archived).fetch_all(&self.pool).await.map_err(anyhow::Error::from)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(self.application_view(row).await?);
        }
        Ok(result)
    }
    pub async fn get_application_organization_id(&self, id: &str) -> anyhow::Result<Option<i64>> {
        let Ok(id) = id.parse::<ApplicationId>() else {
            return Ok(None);
        };
        Ok(
            sqlx::query_scalar("SELECT org_id FROM applications WHERE id=$1")
                .bind(id.uuid())
                .fetch_optional(&self.pool)
                .await?,
        )
    }
    /// All mutation commands already run in one command transaction. The row
    /// lock and expected version prevent lost updates to the resource set.
    pub async fn save_application(
        &self,
        org: i64,
        id: ApplicationId,
        input: ApplicationInput,
        expected: Option<i64>,
    ) -> Result<Application, CommandError> {
        let mut tx = self.pool.begin().await.map_err(anyhow::Error::from)?;
        let current: Option<ApplicationRow> =
            sqlx::query_as("SELECT * FROM applications WHERE org_id=$1 AND id=$2 FOR UPDATE")
                .bind(org)
                .bind(id.uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(anyhow::Error::from)?;
        let published = current.as_ref().is_some_and(|r| r.status == "published");
        match (&current, expected) {
            (Some(row), Some(v)) if row.version == v && row.status != "archived" => {
                if row.slug != input.slug {
                    return Err(CommandError::bad_request(
                        "An Application's stable name cannot be changed",
                    ));
                }
            }
            (None, None) => {}
            (None, Some(_)) => return Err(CommandError::not_found("Application")),
            _ => {
                return Err(CommandError::conflict(
                    "Application changed; reload before saving",
                ));
            }
        }
        if published && input.resources.is_empty() {
            return Err(CommandError::bad_request(
                "A published Application needs at least one resource",
            ));
        }
        let row: ApplicationRow = if expected.is_none() {
            sqlx::query_as("INSERT INTO applications (id,org_id,slug,name,description,config) VALUES ($1,$2,$3,$4,$5,$6) RETURNING *")
                .bind(id.uuid()).bind(org).bind(&input.slug).bind(&input.name).bind(&input.description).bind(serde_json::to_value(&input.config).map_err(anyhow::Error::from)?).fetch_one(&mut *tx).await.map_err(application_conflict)?
        } else {
            sqlx::query_as("UPDATE applications SET name=$3,description=$4,config=$5,version=version+1,updated_at=now() WHERE org_id=$1 AND id=$2 RETURNING *")
                .bind(org).bind(id.uuid()).bind(&input.name).bind(&input.description).bind(serde_json::to_value(&input.config).map_err(anyhow::Error::from)?).fetch_one(&mut *tx).await.map_err(anyhow::Error::from)?
        };
        sqlx::query("DELETE FROM application_resources WHERE application_id=$1")
            .bind(id.uuid())
            .execute(&mut *tx)
            .await
            .map_err(anyhow::Error::from)?;
        for (position, r) in input.resources.iter().enumerate() {
            let (channel, room, memory) = match r.kind {
                ApplicationResourceKind::Chat => {
                    let public_id = r
                        .resource_id
                        .parse::<AgentChannelId>()
                        .map_err(|_| CommandError::bad_request("Invalid channel ID"))?;
                    let key:Option<Uuid>=sqlx::query_scalar("SELECT c.id FROM agent_channels c JOIN agents a ON a.id=c.agent_id WHERE c.public_id=$1 AND a.org_id=$2 AND c.channel_type='work_chat' AND a.status='active' AND (NOT $3 OR (c.status='live' AND c.enabled AND NOT a.exposures_suspended))").bind(public_id.to_string()).bind(org).bind(published).fetch_optional(&mut *tx).await.map_err(anyhow::Error::from)?;
                    let key = key.ok_or_else(|| {
                        CommandError::bad_request(
                            "Chat must be an available Work Chat in this organization",
                        )
                    })?;
                    (Some(key), None, None)
                }
                ApplicationResourceKind::Room => {
                    let key = r
                        .resource_id
                        .parse::<RoomId>()
                        .map_err(|_| CommandError::bad_request("Invalid room ID"))?
                        .uuid();
                    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM rooms WHERE id=$1 AND org_id=$2 AND archived_at IS NULL)").bind(key).bind(org).fetch_one(&mut *tx).await.map_err(anyhow::Error::from)?;
                    if !valid {
                        return Err(CommandError::bad_request(
                            "Room is unavailable in this organization",
                        ));
                    }
                    (None, Some(key), None)
                }
                ApplicationResourceKind::Memory => {
                    let public_id = r
                        .resource_id
                        .parse::<MemoryId>()
                        .map_err(|_| CommandError::bad_request("Invalid memory ID"))?;
                    let key:Option<Uuid>=sqlx::query_scalar("SELECT id FROM memories WHERE public_id=$1 AND org_id=$2 AND scope='org' AND status='active' AND (NOT $3 OR NOT is_readonly)").bind(public_id.to_string()).bind(org).bind(r.writable).fetch_optional(&mut *tx).await.map_err(anyhow::Error::from)?;
                    let key = key.ok_or_else(|| {
                        CommandError::bad_request(
                            "Memory must be an active organization memory with compatible access",
                        )
                    })?;
                    (None, None, Some(key))
                }
            };
            sqlx::query("INSERT INTO application_resources(application_id,slug,name,position,channel_id,room_id,memory_id,writable) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(id.uuid()).bind(&r.slug).bind(&r.name).bind(position as i32).bind(channel).bind(room).bind(memory).bind(r.writable).execute(&mut *tx).await.map_err(application_conflict)?;
        }
        tx.commit().await.map_err(anyhow::Error::from)?;
        self.application_view(row).await.map_err(Into::into)
    }
    pub async fn set_application_status(
        &self,
        org: i64,
        id: ApplicationId,
        version: i64,
        status: &str,
    ) -> Result<Application, CommandError> {
        let row=sqlx::query_as("UPDATE applications SET status=$4,version=version+1,updated_at=now() WHERE org_id=$1 AND id=$2 AND version=$3 AND (status<>'archived' OR $4='draft') RETURNING *")
            .bind(org).bind(id.uuid()).bind(version).bind(status).fetch_optional(&self.pool).await.map_err(anyhow::Error::from)?.ok_or_else(||CommandError::conflict("Application changed or unavailable; reload before publishing"))?;
        self.application_view(row).await.map_err(Into::into)
    }
}
fn application_conflict(error: sqlx::Error) -> CommandError {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation())
    {
        CommandError::conflict("This stable name or resource is already assigned to an Application")
    } else {
        anyhow::Error::from(error).into()
    }
}

impl Database {
    /// A caller cannot choose or impersonate a Room member. Concurrent first
    /// joins serialize on the Application and reuse this server-owned mapping.
    pub async fn application_room_member(
        &self,
        org: i64,
        application: ApplicationId,
        user: everruns_contracts::typed_id::VirtualUserId,
        room: RoomId,
        name: &str,
    ) -> anyhow::Result<everruns_contracts::typed_id::RoomMemberId> {
        let mut tx = self.pool.begin().await.map_err(anyhow::Error::from)?;
        sqlx::query("SELECT id FROM applications WHERE id=$1 AND org_id=$2 FOR UPDATE")
            .bind(application.uuid())
            .bind(org)
            .execute(&mut *tx)
            .await
            .map_err(anyhow::Error::from)?;
        let existing:Option<(Uuid,Option<DateTime<Utc>>)>=sqlx::query_as("SELECT m.id,m.left_at FROM application_room_members b JOIN room_members m ON m.id=b.member_id WHERE b.application_id=$1 AND b.virtual_user_id=$2 AND b.room_id=$3")
            .bind(application.uuid()).bind(user.uuid()).bind(room.uuid()).fetch_optional(&mut *tx).await.map_err(anyhow::Error::from)?;
        if let Some((id, left)) = existing {
            if left.is_some() {
                anyhow::bail!("Room membership revoked");
            }
            return Ok(everruns_contracts::typed_id::RoomMemberId::from_uuid(id));
        }
        let member = Uuid::now_v7();
        sqlx::query("INSERT INTO room_members(id,room_id,org_id,kind,handle,display_name,role,wake) VALUES($1,$2,$3,'guest',$4,$5,'member','never')")
            .bind(member).bind(room.uuid()).bind(org).bind(format!("u{}",user.uuid().simple())).bind(name.chars().take(120).collect::<String>()).execute(&mut *tx).await.map_err(anyhow::Error::from)?;
        sqlx::query("INSERT INTO application_room_members(application_id,virtual_user_id,room_id,member_id) VALUES($1,$2,$3,$4)")
            .bind(application.uuid()).bind(user.uuid()).bind(room.uuid()).bind(member).execute(&mut *tx).await.map_err(anyhow::Error::from)?;
        tx.commit().await.map_err(anyhow::Error::from)?;
        Ok(everruns_contracts::typed_id::RoomMemberId::from_uuid(
            member,
        ))
    }
}

impl Database {
    pub async fn application_for_session(
        &self,
        org: i64,
        session: everruns_contracts::typed_id::SessionId,
    ) -> anyhow::Result<Option<ApplicationRow>> {
        Ok(sqlx::query_as("WITH RECURSIVE ancestors AS (SELECT id,channel_id,parent_session_id,0 AS depth FROM sessions WHERE org_id=$1 AND id=$2 UNION ALL SELECT s.id,s.channel_id,s.parent_session_id,p.depth+1 FROM sessions s JOIN ancestors p ON s.id=p.parent_session_id WHERE s.org_id=$1 AND p.depth<32) SELECT a.* FROM applications a JOIN application_resources r ON r.application_id=a.id WHERE a.org_id=$1 AND (r.channel_id IN (SELECT channel_id FROM ancestors) OR r.room_id IN (SELECT room_id FROM room_agent_sessions WHERE session_id IN (SELECT id FROM ancestors))) LIMIT 1")
            .bind(org).bind(session.uuid()).fetch_optional(&self.pool).await?)
    }
}
