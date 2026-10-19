//! Common ownership/grant authorization; provider adapters enforce narrower use.
use crate::domains::common::{CommandError, Ctx};
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::credential_services::CredentialInvocation;
use everruns_contracts::runtime::resource_access::*;
use everruns_contracts::typed_id::{AgentId, VirtualUserId};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct AccessRecord {
    pub id: Uuid,
    pub org_id: i64,
    pub kind: String,
    pub scope: String,
    pub owner_agent_id: Option<Uuid>,
    pub owner_session_id: Option<Uuid>,
    pub owner_virtual_user_id: Option<Uuid>,
    pub subject_id: Option<Uuid>,
    pub usage: String,
    pub revision: i64,
    pub revoked_at: Option<DateTime<Utc>>,
}

pub fn scope(value: &str) -> Result<ResourceScope, CommandError> {
    serde_json::from_value(serde_json::json!(value))
        .map_err(|_| CommandError::bad_request("Invalid resource scope"))
}
pub fn usage(value: &str) -> Result<CredentialUsage, CommandError> {
    serde_json::from_value(serde_json::json!(value))
        .map_err(|_| CommandError::bad_request("Invalid credential usage"))
}
pub fn wire<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl AccessRecord {
    pub fn owner(&self) -> Result<ResourceOwner, CommandError> {
        Ok(ResourceOwner {
            org_id: self.org_id,
            scope: scope(&self.scope)?,
            owner_id: self
                .owner_agent_id
                .or(self.owner_session_id)
                .or(self.owner_virtual_user_id),
            subject_id: self.subject_id,
            usage: usage(&self.usage)?,
        })
    }
}

/// THREAT[TM-AUTHZ-023]: only persisted invocation provenance supplies authority.
pub async fn actor(
    ctx: &Ctx,
    invocation: CredentialInvocation,
    selected: CredentialUsage,
) -> Result<ResourceActor, CommandError> {
    let session = ctx
        .db
        .get_session(ctx.org_id(), invocation.session_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    if !ctx
        .db
        .runtime_invocation_exists(invocation.session_id, invocation.input_message_id)
        .await?
    {
        return Err(CommandError::forbidden("Unknown runtime invocation"));
    }
    let mut agent_id = ctx
        .db
        .runtime_invocation_responder(invocation.session_id, invocation.input_message_id)
        .await?;
    let agent = if let Some(id) = agent_id {
        ctx.db
            .get_agent(ctx.org_id(), AgentId::from_uuid(id))
            .await?
            .filter(|a| a.status == "active")
    } else {
        None
    };
    if agent.is_none() {
        agent_id = None;
    }
    let service = agent.and_then(|a| a.virtual_user_id);
    let expected_service = service.is_some();
    let service = if let Some(id) = service {
        ctx.db
            .get_virtual_user(ctx.org_id(), id)
            .await?
            .filter(|v| v.status == "active" && v.usage == "service")
            .map(|v| v.id.uuid())
    } else {
        None
    };
    if expected_service && service.is_none() {
        agent_id = None;
    }
    let subject = ctx
        .db
        .runtime_invocation_subject(invocation.session_id, invocation.input_message_id)
        .await?
        .map(|id| id.uuid());
    let consumers: i64 = sqlx::query_scalar("SELECT count(DISTINCT virtual_user_id) FROM runtime_invocations WHERE org_id=$1 AND session_id=$2")
        .bind(ctx.org_id()).bind(invocation.session_id).fetch_one(ctx.db.pool()).await.map_err(|e|CommandError::internal(e.into()))?;
    let private_access = !ctx.db.is_playground_session(invocation.session_id).await?
        && session.workspace_id == invocation.session_id.uuid()
        && consumers <= 1;
    let result = ResourceActor {
        org_id: ctx.org_id(),
        session_id: invocation.session_id.uuid(),
        agent_id,
        virtual_user_id: subject,
        service_user_id: service,
        usage: selected,
        private_access,
    };
    if selected == CredentialUsage::User && result.subject().is_none() {
        return Err(CommandError::forbidden(
            "A verified user in a private execution context is required",
        ));
    }
    Ok(result)
}

pub async fn record(ctx: &Ctx, id: Uuid) -> Result<AccessRecord, CommandError> {
    sqlx::query_as(
            r#"
            SELECT id,org_id,kind,scope,owner_agent_id,owner_session_id,owner_virtual_user_id,subject_id,usage,revision,revoked_at FROM resource_access
            WHERE org_id=$1 AND id=$2
            "#,
        )
        .bind(ctx.org_id()).bind(id).fetch_optional(ctx.db.pool()).await.map_err(|e|CommandError::internal(e.into()))?
        .ok_or_else(||CommandError::not_found("Resource"))
}

#[derive(sqlx::FromRow)]
struct GrantRow {
    resource_id: Uuid,
    org_id: i64,
    consumer_scope: String,
    consumer_agent_id: Option<Uuid>,
    consumer_session_id: Option<Uuid>,
    consumer_virtual_user_id: Option<Uuid>,
    consumer_subject_id: Option<Uuid>,
    constraints: Option<serde_json::Value>,
    usage: String,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

pub async fn permits(
    ctx: &Ctx,
    record: &AccessRecord,
    actor: &ResourceActor,
) -> Result<bool, CommandError> {
    permits_github(ctx, record, actor, None).await
}

pub async fn permits_github(
    ctx: &Ctx,
    record: &AccessRecord,
    actor: &ResourceActor,
    request: Option<&everruns_contracts::runtime::GitHubAccess>,
) -> Result<bool, CommandError> {
    if record.org_id != actor.org_id || record.revoked_at.is_some() {
        return Ok(false);
    }
    if record.owner()?.permits_owner(actor) {
        return Ok(true);
    }
    let grants: Vec<GrantRow> = sqlx::query_as(
            r#"
            SELECT resource_id,org_id,consumer_scope,consumer_agent_id,consumer_session_id,consumer_virtual_user_id,consumer_subject_id,constraints,usage,expires_at,revoked_at FROM resource_grants
            WHERE org_id=$1 AND resource_id=$2 AND revoked_at IS NULL
            LIMIT 257
            "#,
        )
        .bind(ctx.org_id()).bind(record.id).fetch_all(ctx.db.pool()).await.map_err(|e|CommandError::internal(e.into()))?;
    if grants.len() > 256 {
        return Err(CommandError::bad_request("Too many resource grants"));
    }
    for g in grants {
        let grant = ResourceGrant {
            org_id: g.org_id,
            consumer_scope: scope(&g.consumer_scope)?,
            consumer_id: g
                .consumer_agent_id
                .or(g.consumer_session_id)
                .or(g.consumer_virtual_user_id),
            subject_id: g.consumer_subject_id,
            usage: usage(&g.usage)?,
            expires_at: g.expires_at,
            revoked_at: g.revoked_at,
        };
        let constraints = match g.constraints {
            None => true,
            Some(v) => serde_json::from_value::<ResourceConstraints>(v)
                .ok()
                .is_some_and(|c| request.is_some_and(|r| c.permits_github(r))),
        };
        if constraints && grant.permits(actor, Utc::now()) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Management of a private account is not authority to provision its secrets.
pub async fn require_manage(ctx: &Ctx, resource: &AccessRecord) -> Result<(), CommandError> {
    use everruns_core::{Permission, Policy, Rule};
    match scope(&resource.scope)? {
        ResourceScope::VirtualUser => {
            let id = resource
                .owner_virtual_user_id
                .ok_or_else(|| CommandError::forbidden("Invalid owner"))?;
            crate::domains::virtual_users::connection_target(
                &ctx.db,
                ctx.permission_resolver.as_ref(),
                &ctx.caller,
                &VirtualUserId::from_uuid(id).to_string(),
            )
            .await?;
        }
        ResourceScope::Agent => crate::domains::agents::AGENT_MANAGE
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|_| CommandError::forbidden("Agent management required"))?,
        ResourceScope::Organization => Policy {
            id: "resource.manage",
            rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
        }
        .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
        .map_err(|_| CommandError::forbidden("Organization management required"))?,
        ResourceScope::Session => {
            crate::domains::sessions::SESSION_MANAGE
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .map_err(|_| CommandError::forbidden("Session management required"))?;
            if let Some(subject) = resource.subject_id {
                crate::domains::virtual_users::connection_target(
                    &ctx.db,
                    ctx.permission_resolver.as_ref(),
                    &ctx.caller,
                    &VirtualUserId::from_uuid(subject).to_string(),
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// Keep secret values and capture arguments out of configuration history.
pub async fn audit(
    ctx: &Ctx,
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    action: &'static str,
    input: Option<Uuid>,
) -> Result<(), CommandError> {
    sqlx::query("INSERT INTO resource_access_audit(org_id,resource_id,action,actor_user_id,input_message_id) VALUES($1,$2,$3,$4,$5)")
        .bind(ctx.org_id())
        .bind(id)
        .bind(action)
        .bind(ctx.caller.user_id)
        .bind(input)
        .execute(tx).await
        .map_err(|e|CommandError::internal(e.into()))?;
    Ok(())
}

/// A credential list is bounded to 256 rows. Fetch all headers and matching
/// grants in two queries rather than doing two lookups for every binding.
pub async fn authorized_records(
    ctx: &Ctx,
    ids: &[Uuid],
    actor: &ResourceActor,
) -> Result<std::collections::HashMap<Uuid, AccessRecord>, CommandError> {
    if ids.len() > 256 {
        return Err(CommandError::bad_request("Too many resources"));
    }
    let rows:Vec<AccessRecord>=sqlx::query_as(
            r#"
            SELECT id,org_id,kind,scope,owner_agent_id,owner_session_id,owner_virtual_user_id,subject_id,usage,revision,revoked_at FROM resource_access
            WHERE org_id=$1 AND id=ANY($2) AND revoked_at IS NULL
            "#,
        )
        .bind(ctx.org_id())
        .bind(ids)
        .fetch_all(ctx.db.pool()).await
        .map_err(|e|CommandError::internal(e.into()))?;
    let grants:Vec<GrantRow>=sqlx::query_as(
            r#"
            SELECT resource_id,org_id,consumer_scope,consumer_agent_id,consumer_session_id,consumer_virtual_user_id,consumer_subject_id,constraints,usage,expires_at,revoked_at FROM resource_grants
            WHERE org_id=$1 AND resource_id=ANY($2) AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>now()) AND usage=$3 AND constraints IS NULL AND (consumer_agent_id=$4 OR consumer_virtual_user_id=$5 OR consumer_session_id=$6 OR consumer_scope='organization')
            LIMIT 65537
            "#,
        )
        .bind(ctx.org_id())
        .bind(ids)
        .bind(wire(actor.usage))
        .bind(actor.agent_id)
        .bind(actor.subject())
        .bind(actor.session_id)
        .fetch_all(ctx.db.pool()).await
        .map_err(|e|CommandError::internal(e.into()))?;
    if grants.len() > 65536 {
        return Err(CommandError::bad_request("Too many resource grants"));
    }
    let now = Utc::now();
    let mut allowed = std::collections::HashSet::new();
    for g in grants {
        let grant = ResourceGrant {
            org_id: g.org_id,
            consumer_scope: scope(&g.consumer_scope)?,
            consumer_id: g
                .consumer_agent_id
                .or(g.consumer_session_id)
                .or(g.consumer_virtual_user_id),
            subject_id: g.consumer_subject_id,
            usage: usage(&g.usage)?,
            expires_at: g.expires_at,
            revoked_at: g.revoked_at,
        };
        if grant.permits(actor, now) {
            allowed.insert(g.resource_id);
        }
    }
    let mut result = std::collections::HashMap::new();
    for r in rows {
        if r.org_id == actor.org_id && (r.owner()?.permits_owner(actor) || allowed.contains(&r.id))
        {
            result.insert(r.id, r);
        }
    }
    Ok(result)
}
