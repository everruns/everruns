//! Shared management service. Values are accepted only by secure HTTP entry.
use super::service::owner_key;
use crate::domains::common::{CommandError, Ctx};
use crate::domains::resource_access::{self as access, AccessRecord};
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::resource_access::*;
use everruns_contracts::runtime::{SecretBinding, credential_http::validate_name};
use everruns_contracts::typed_id::{AgentId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

fn error(e: sqlx::Error) -> CommandError {
    CommandError::internal(e.into())
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CredentialOwnerInput {
    pub scope: ResourceScope,
    pub owner_id: Option<String>,
    pub usage: CredentialUsage,
}

pub async fn owner(ctx: &Ctx, input: &CredentialOwnerInput) -> Result<AccessRecord, CommandError> {
    let mut r = AccessRecord {
        id: Uuid::now_v7(),
        org_id: ctx.org_id(),
        kind: "credential_binding".into(),
        scope: access::wire(input.scope),
        owner_agent_id: None,
        owner_session_id: None,
        owner_virtual_user_id: None,
        subject_id: None,
        usage: access::wire(input.usage),
        revision: 1,
        revoked_at: None,
    };
    match input.scope {
        ResourceScope::Organization => {
            if input.owner_id.is_some() || input.usage != CredentialUsage::Service {
                return Err(CommandError::bad_request(
                    "Organization credentials require service usage and no owner ID",
                ));
            }
        }
        ResourceScope::Agent => {
            if input.usage != CredentialUsage::Service {
                return Err(CommandError::bad_request(
                    "Agent credentials require service usage",
                ));
            }
            let id: AgentId = input
                .owner_id
                .as_deref()
                .unwrap_or_default()
                .parse()
                .map_err(|_| CommandError::bad_request("Invalid agent owner"))?;
            let agent = ctx
                .db
                .get_agent(ctx.org_id(), id)
                .await?
                .filter(|a| a.status == "active")
                .ok_or_else(|| CommandError::not_found("Agent"))?;
            r.owner_agent_id = Some(agent.id.uuid());
        }
        ResourceScope::VirtualUser => {
            let id = crate::domains::virtual_users::connection_target(
                &ctx.db,
                ctx.permission_resolver.as_ref(),
                &ctx.caller,
                input.owner_id.as_deref().unwrap_or("me"),
            )
            .await?;
            let v = ctx
                .db
                .get_virtual_user(ctx.org_id(), id)
                .await?
                .ok_or_else(|| CommandError::not_found("Virtual user"))?;
            if (input.usage == CredentialUsage::User) != (v.usage == "end_user") {
                return Err(CommandError::bad_request(
                    "Credential usage does not match the virtual user",
                ));
            }
            r.owner_virtual_user_id = Some(id.uuid());
        }
        ResourceScope::Session => {
            let id: SessionId = input
                .owner_id
                .as_deref()
                .unwrap_or_default()
                .parse()
                .map_err(|_| CommandError::bad_request("Invalid session owner"))?;
            ctx.db
                .get_session(ctx.org_id(), id)
                .await?
                .ok_or_else(|| CommandError::not_found("Session"))?;
            r.owner_session_id = Some(id.uuid());
            if input.usage == CredentialUsage::User {
                let v = crate::domains::virtual_users::connection_target(
                    &ctx.db,
                    ctx.permission_resolver.as_ref(),
                    &ctx.caller,
                    "me",
                )
                .await?;
                r.subject_id = Some(v.uuid());
            }
        }
    }
    access::require_manage(ctx, &r).await?;
    Ok(r)
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ManagedCredential {
    pub id: Uuid,
    pub name: Option<String>,
    pub label: String,
    pub kind: String,
    pub binding: Option<Value>,
    pub mcp_server_url: Option<String>,
    pub tool_name: Option<String>,
    pub parameter_name: Option<String>,
    pub scope: String,
    pub configured: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub revision: i64,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_used_agent_id: Option<Uuid>,
    pub last_used_agent_name: Option<String>,
    pub source_connection_id: Option<Uuid>,
}

pub async fn list(
    ctx: &Ctx,
    input: &CredentialOwnerInput,
) -> Result<Vec<ManagedCredential>, CommandError> {
    let r = owner(ctx, input).await?;
    let key = owner_key(
        input.scope,
        r.owner_agent_id
            .or(r.owner_session_id)
            .or(r.owner_virtual_user_id),
        r.subject_id,
        input.usage,
        ctx.org_id(),
    );
    let rows:Vec<ManagedCredential>=sqlx::query_as(
            r#"
            SELECT c.id,c.env_name AS name,c.label,c.kind,c.binding,c.mcp_server_url,c.tool_name,c.parameter_name,r.scope,(c.value_encrypted IS NOT NULL OR c.source_connection_id IS NOT NULL) AS configured,c.expires_at,r.revision,c.last_used_at,c.last_used_agent_id,a.name AS last_used_agent_name,c.source_connection_id FROM credential_bindings c
            JOIN resource_access r ON r.id=c.id AND r.org_id=c.org_id
            LEFT
            JOIN agents a ON a.id=c.last_used_agent_id AND a.org_id=c.org_id
            WHERE c.org_id=$1 AND (c.owner_key=$2 OR ($3::uuid IS NOT NULL AND r.owner_session_id=$3 AND r.usage='service')) AND r.revoked_at IS NULL
            ORDER BY c.created_at DESC
            LIMIT 257
            "#,
        )
        .bind(ctx.org_id())
        .bind(key)
        .bind(if input.scope==ResourceScope::Session&&input.usage==CredentialUsage::Service{r.owner_session_id}else{None})
        .fetch_all(ctx.db.pool()).await
        .map_err(error)?;
    if rows.len() > 256 {
        return Err(CommandError::bad_request(
            "Too many credentials; narrow the owner",
        ));
    }
    Ok(rows)
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateScopedCredential {
    #[serde(flatten)]
    pub owner: CredentialOwnerInput,
    pub name: Option<String>,
    pub label: String,
    pub binding: Option<SecretBinding>,
    pub mcp: Option<McpCredentialTarget>,
    pub source: Option<ConnectionCredentialSource>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionCredentialSource {
    pub connection_id: Uuid,
    pub github_access: everruns_contracts::runtime::GitHubAccess,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct McpCredentialTarget {
    pub server_name: String,
    pub url: String,
    pub tool_name: String,
    pub parameter_name: String,
}

pub async fn create(ctx: &Ctx, input: CreateScopedCredential) -> Result<Uuid, CommandError> {
    if input.label.trim().is_empty() || input.label.len() > 128 {
        return Err(CommandError::bad_request(
            "Use a valid credential name and a label up to 128 characters",
        ));
    }
    let (binding, mcp) = match (input.binding, input.mcp) {
        (Some(mut b), None) => {
            if !input.name.as_deref().is_some_and(validate_name) {
                return Err(CommandError::bad_request(
                    "HTTP credentials require a valid reference name",
                ));
            }
            b.url = b.validate().map_err(CommandError::bad_request)?;
            (Some(b), None)
        }
        (None, Some(m)) => {
            if [&m.server_name, &m.tool_name, &m.parameter_name]
                .iter()
                .any(|v| v.trim().is_empty() || v.len() > 255)
                || m.parameter_name.contains('.')
            {
                return Err(CommandError::bad_request(
                    "Use a server, tool, and top-level parameter name",
                ));
            }
            let mut target = SecretBinding {
                url: m.url.clone(),
                auth: everruns_contracts::runtime::SecretAuth::Bearer,
            };
            target.url = target.validate().map_err(CommandError::bad_request)?;
            (
                None,
                Some(McpCredentialTarget {
                    url: url::Url::parse(&m.url)
                        .map_err(|_| CommandError::bad_request("Invalid MCP URL"))?
                        .to_string(),
                    ..m
                }),
            )
        }
        _ => {
            return Err(CommandError::bad_request(
                "Choose one HTTP or MCP parameter target",
            ));
        }
    };
    if let Some(source) = &input.source {
        let source_record = access::record(ctx, source.connection_id).await?;
        access::require_manage(ctx, &source_record).await?;
        let kind:Option<String>=sqlx::query_scalar("SELECT connection_type FROM virtual_user_connections WHERE id=$1 AND org_id=$2 AND provider='github'")
        .bind(source.connection_id)
        .bind(ctx.org_id())
        .fetch_optional(ctx.db.pool()).await
        .map_err(error)?;
        let url = binding.as_ref().and_then(|b| url::Url::parse(&b.url).ok());
        let constraint = ResourceConstraints::Github {
            repositories: source.github_access.repositories.clone(),
            permissions: source.github_access.permissions.clone(),
        };
        if source_record.kind != "connection"
            || source_record.revoked_at.is_some()
            || !matches!(kind.as_deref(), Some("github_app" | "everruns_app"))
            || url.as_ref().is_none_or(|u| {
                u.scheme() != "https"
                    || u.host_str() != Some("api.github.com")
                    || u.port_or_known_default() != Some(443)
            })
            || !constraint.validate()
            || !matches!(
                binding.as_ref().map(|b| &b.auth),
                Some(everruns_contracts::runtime::SecretAuth::Bearer)
            )
            || source.github_access.acts_as != everruns_core::McpServerActsAs::Service
        {
            return Err(CommandError::bad_request(
                "Connection-backed HTTP bindings require a GitHub App, api.github.com bearer placement, and explicit service repository/permission limits",
            ));
        }
    }
    let r = owner(ctx, &input.owner).await?;
    let mut tx = ctx.db.pool().begin().await.map_err(error)?;
    sqlx::query(
            r#"
            INSERT INTO resource_access(id,org_id,kind,scope,owner_agent_id,owner_session_id,owner_virtual_user_id,subject_id,usage)
            VALUES($1,$2,'credential_binding',$3,$4,$5,$6,$7,$8)
            "#,
        )
        .bind(r.id)
        .bind(r.org_id)
        .bind(&r.scope)
        .bind(r.owner_agent_id)
        .bind(r.owner_session_id)
        .bind(r.owner_virtual_user_id)
        .bind(r.subject_id)
        .bind(&r.usage)
        .execute(&mut *tx).await
        .map_err(error)?;
    let result=sqlx::query(
            r#"
            INSERT INTO credential_bindings(id,org_id,agent_id,kind,env_name,binding,label,mcp_server_name,mcp_server_url,tool_name,parameter_name,source_connection_id,source_github_access)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            "#,
        )
        .bind(r.id)
        .bind(r.org_id)
        .bind(r.owner_agent_id)
        .bind(if mcp.is_some(){"mcp_parameter"}else{"http"})
        .bind(if mcp.is_none(){input.name}else{None})
        .bind(binding.map(|b|json!(b)))
        .bind(input.label)
        .bind(mcp.as_ref().map(|m|&m.server_name))
        .bind(mcp.as_ref().map(|m|&m.url))
        .bind(mcp.as_ref().map(|m|&m.tool_name))
        .bind(mcp.as_ref().map(|m|&m.parameter_name))
        .bind(input.source.as_ref().map(|s|s.connection_id))
        .bind(input.source.as_ref().map(|s|json!(s.github_access)))
        .execute(&mut *tx).await;
    if result.as_ref().is_err_and(|e| {
        e.as_database_error()
            .is_some_and(|e| e.is_unique_violation())
    }) {
        return Err(CommandError::bad_request(
            "A credential with this target already exists for this owner",
        ));
    }
    result.map_err(error)?;
    access::audit(ctx, &mut tx, r.id, "created", None).await?;
    tx.commit().await.map_err(error)?;
    Ok(r.id)
}

pub async fn set_value(
    ctx: &Ctx,
    id: Uuid,
    value: String,
    expires_at: Option<DateTime<Utc>>,
    revision: i64,
) -> Result<(), CommandError> {
    if ctx.caller.read_only {
        return Err(CommandError::forbidden(
            "Read-only credentials cannot provision values",
        ));
    }
    let record = access::record(ctx, id).await?;
    access::require_manage(ctx, &record).await?;
    if record.kind != "credential_binding"
        || value.is_empty()
        || value.len() > 64 * 1024
        || expires_at.is_some_and(|v| v <= Utc::now())
    {
        return Err(CommandError::bad_request(
            "Invalid credential value or expiry",
        ));
    }
    let encrypted = ctx
        .encryption
        .as_deref()
        .ok_or_else(|| CommandError::unavailable("Credential encryption unavailable"))?
        .encrypt_string(&value)
        .map_err(|_| CommandError::unavailable("Credential encryption failed"))?;
    let mut tx = ctx.db.pool().begin().await.map_err(error)?;
    let live:Option<i64>=sqlx::query_scalar("SELECT revision FROM resource_access WHERE id=$1 AND org_id=$2 AND revoked_at IS NULL FOR UPDATE")
        .bind(id)
        .bind(ctx.org_id())
        .fetch_optional(&mut *tx).await
        .map_err(error)?;
    if live != Some(revision) {
        return Err(CommandError::bad_request(
            "Credential changed; refresh before replacing it",
        ));
    }
    sqlx::query(
        r#"
            UPDATE credential_bindings
            SET value_encrypted=$2,source_connection_id=NULL,expires_at=$3,updated_at=now()
            WHERE id=$1 AND org_id=$4
            "#,
    )
    .bind(id)
    .bind(encrypted)
    .bind(expires_at)
    .bind(ctx.org_id())
    .execute(&mut *tx)
    .await
    .map_err(error)?;
    access::audit(ctx, &mut tx, id, "value_changed", None).await?;
    tx.commit().await.map_err(error)?;
    Ok(())
}

pub async fn revoke(ctx: &Ctx, id: Uuid, revision: i64) -> Result<(), CommandError> {
    let record = access::record(ctx, id).await?;
    access::require_manage(ctx, &record).await?;
    if record.kind != "credential_binding" {
        return Err(CommandError::not_found("Credential"));
    }
    let mut tx = ctx.db.pool().begin().await.map_err(error)?;
    let updated = sqlx::query(
        r#"
            UPDATE resource_access
            SET revoked_at=now(),revision=revision+1
            WHERE org_id=$1 AND id=$2 AND revision=$3 AND revoked_at IS NULL
            "#,
    )
    .bind(ctx.org_id())
    .bind(id)
    .bind(revision)
    .execute(&mut *tx)
    .await
    .map_err(error)?;
    if updated.rows_affected() != 1 {
        return Err(CommandError::bad_request(
            "Credential changed; refresh before revoking it",
        ));
    }
    sqlx::query("DELETE FROM credential_bindings WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(error)?;
    sqlx::query("UPDATE resource_grants SET revoked_at=now(),revision=revision+1 WHERE resource_id=$1 AND org_id=$2")
        .bind(id)
        .bind(ctx.org_id())
        .execute(&mut *tx).await
        .map_err(error)?;
    access::audit(ctx, &mut tx, id, "revoked", None).await?;
    tx.commit().await.map_err(error)?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GrantCredential {
    pub constraints: Option<ResourceConstraints>,
    pub consumer_scope: ResourceScope,
    pub consumer_id: Option<String>,
    pub usage: CredentialUsage,
    pub expires_at: Option<DateTime<Utc>>,
}

pub async fn grant(ctx: &Ctx, id: Uuid, input: GrantCredential) -> Result<Uuid, CommandError> {
    // Grant administration is an explicit owner action. An agent with ambient
    // management permissions cannot turn ordinary configuration into sharing.
    if ctx.acting_for_session.is_some()
        || ctx.change_intent.as_ref().is_some_and(|i| {
            i.via_session_id.is_some()
                || matches!(
                    i.surface,
                    Some(
                        crate::domains::change_history::ChangeSurface::Platform
                            | crate::domains::change_history::ChangeSurface::Worker
                    )
                )
        })
    {
        return Err(CommandError::forbidden(
            "Grant access from Credentials settings; an agent runtime cannot create sharing grants",
        ));
    }
    let r = access::record(ctx, id).await?;
    access::require_manage(ctx, &r).await?;
    if r.revoked_at.is_some() || input.expires_at.is_some_and(|e| e <= Utc::now()) {
        return Err(CommandError::bad_request(
            "Resource or grant expired/revoked",
        ));
    }
    if input.constraints.as_ref().is_some_and(|c| !c.validate()) {
        return Err(CommandError::bad_request(
            "Invalid provider access constraints",
        ));
    }
    if input.constraints.is_some() {
        let provider: Option<String> =
            sqlx::query_scalar("SELECT provider FROM virtual_user_connections WHERE id=$1")
                .bind(id)
                .fetch_optional(ctx.db.pool())
                .await
                .map_err(error)?;
        if r.kind != "connection" || provider.as_deref() != Some("github") {
            return Err(CommandError::bad_request(
                "GitHub constraints require a GitHub connection",
            ));
        }
    }
    let consumer = owner(
        ctx,
        &CredentialOwnerInput {
            scope: input.consumer_scope,
            owner_id: input.consumer_id,
            usage: input.usage,
        },
    )
    .await?;
    let mut tx = ctx.db.pool().begin().await.map_err(error)?;
    let live:Option<Uuid>=sqlx::query_scalar("SELECT id FROM resource_access WHERE id=$1 AND org_id=$2 AND revoked_at IS NULL FOR UPDATE")
        .bind(id)
        .bind(ctx.org_id())
        .fetch_optional(&mut *tx).await
        .map_err(error)?;
    if live.is_none() {
        return Err(CommandError::not_found("Resource"));
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM resource_grants WHERE org_id=$1 AND resource_id=$2 AND revoked_at IS NULL")
        .bind(ctx.org_id()).bind(id).fetch_one(&mut *tx).await.map_err(error)?;
    if count >= 256 {
        return Err(CommandError::bad_request("Resource grant limit reached"));
    }
    let grant = Uuid::now_v7();
    sqlx::query(
            r#"
            INSERT INTO resource_grants(id,org_id,resource_id,consumer_scope,consumer_agent_id,consumer_session_id,consumer_virtual_user_id,usage,authorizer_user_id,expires_at,consumer_subject_id,constraints)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
            "#,
        )
        .bind(grant)
        .bind(ctx.org_id())
        .bind(id)
        .bind(consumer.scope)
        .bind(consumer.owner_agent_id)
        .bind(consumer.owner_session_id)
        .bind(consumer.owner_virtual_user_id)
        .bind(access::wire(input.usage))
        .bind(ctx.caller.user_id)
        .bind(input.expires_at)
        .bind(consumer.subject_id)
        .bind(input.constraints.map(|v|json!(v)))
        .execute(&mut *tx).await
        .map_err(error)?;
    access::audit(ctx, &mut tx, id, "granted", None).await?;
    tx.commit().await.map_err(error)?;
    Ok(grant)
}

pub async fn grants(ctx: &Ctx, id: Uuid) -> Result<Vec<Value>, CommandError> {
    let r = access::record(ctx, id).await?;
    access::require_manage(ctx, &r).await?;
    sqlx::query_scalar(
            r#"
            SELECT jsonb_build_object('id',g.id,'consumer_scope',consumer_scope,'consumer_id',coalesce(consumer_agent_id,consumer_session_id,consumer_virtual_user_id),'consumer_label',coalesce(a.name,v.name,s.title,'All service agents'),'usage',g.usage,'expires_at',g.expires_at,'constraints',g.constraints) FROM resource_grants g
            LEFT
            JOIN agents a ON a.id=g.consumer_agent_id AND a.org_id=g.org_id
            LEFT
            JOIN virtual_users v ON v.id=g.consumer_virtual_user_id AND v.org_id=g.org_id
            LEFT
            JOIN sessions s ON s.id=g.consumer_session_id AND s.org_id=g.org_id
            WHERE g.org_id=$1 AND resource_id=$2 AND g.revoked_at IS NULL
            ORDER BY g.created_at DESC
            LIMIT 256
            "#,
        )
        .bind(ctx.org_id()).bind(id).fetch_all(ctx.db.pool()).await.map_err(error)
}

pub async fn revoke_grant(ctx: &Ctx, id: Uuid, grant_id: Uuid) -> Result<(), CommandError> {
    let r = access::record(ctx, id).await?;
    access::require_manage(ctx, &r).await?;
    let mut tx = ctx.db.pool().begin().await.map_err(error)?;
    sqlx::query(
        r#"
            UPDATE resource_grants
            SET revoked_at=now(),revision=revision+1
            WHERE id=$1 AND org_id=$2 AND resource_id=$3 AND revoked_at IS NULL
            "#,
    )
    .bind(grant_id)
    .bind(ctx.org_id())
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(error)?;
    access::audit(ctx, &mut tx, id, "grant_revoked", None).await?;
    tx.commit().await.map_err(error)?;
    Ok(())
}
