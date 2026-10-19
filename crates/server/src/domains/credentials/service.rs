use crate::domains::common::{CommandError, Ctx};
use crate::domains::resource_access::{self as access, AccessRecord};
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::SecretBinding;
use everruns_contracts::runtime::credential_http::{same_origin, validate_name};
use everruns_contracts::runtime::credential_services::*;
use everruns_contracts::runtime::resource_access::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn invalid() -> CommandError {
    CommandError::bad_request("Invalid credential request")
}
fn db_error(error: sqlx::Error) -> CommandError {
    CommandError::internal(error.into())
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, CommandError> {
    serde_json::from_value(value).map_err(|_| invalid())
}

#[derive(sqlx::FromRow)]
pub(crate) struct CredentialRow {
    pub id: Uuid,
    pub env_name: Option<String>,
    pub binding: Option<sqlx::types::Json<SecretBinding>>,
    pub value_encrypted: Option<Vec<u8>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub source_connection_id: Option<Uuid>,
    pub source_github_access: Option<Value>,
}

async fn available(
    ctx: &Ctx,
    actor: &ResourceActor,
) -> Result<Vec<(CredentialRow, AccessRecord)>, CommandError> {
    let query = r#"
            SELECT c.id,c.env_name,c.binding,c.value_encrypted,c.expires_at,c.source_connection_id,c.source_github_access FROM credential_bindings c
            JOIN resource_access r ON r.id=c.id AND r.org_id=c.org_id
            WHERE c.org_id=$1 AND c.kind='http' AND r.revoked_at IS NULL AND (r.owner_agent_id=$2 OR r.owner_virtual_user_id=$3 OR r.owner_session_id=$4 OR EXISTS(SELECT 1 FROM resource_grants g
            WHERE g.resource_id=r.id AND g.org_id=r.org_id AND g.revoked_at IS NULL AND (g.consumer_agent_id=$2 OR g.consumer_virtual_user_id=$3 OR g.consumer_session_id=$4 OR g.consumer_scope='organization')))
            ORDER BY c.id
            LIMIT 257
            "#;
    let rows: Vec<CredentialRow> = sqlx::query_as(query)
        .bind(ctx.org_id())
        .bind(actor.agent_id)
        .bind(actor.subject())
        .bind(actor.session_id)
        .fetch_all(ctx.db.pool())
        .await
        .map_err(db_error)?;
    if rows.len() > 256 {
        return Err(CommandError::bad_request("Too many available credentials"));
    }
    let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
    let mut allowed = access::authorized_records(ctx, &ids, actor).await?;
    let mut result = Vec::new();
    for row in rows {
        if let Some(record) = allowed.remove(&row.id) {
            result.push((row, record));
        }
    }

    Ok(result)
}

fn info(row: &CredentialRow, record: &AccessRecord) -> Result<CredentialInfo, CommandError> {
    Ok(CredentialInfo {
        id: row.id,
        revision: record.revision,
        name: row.env_name.clone().ok_or_else(invalid)?,
        binding: row.binding.as_ref().ok_or_else(invalid)?.0.clone(),
        scope: access::scope(&record.scope)?,
        expires_at: row.expires_at,
    })
}

pub async fn execute(
    ctx: &Ctx,
    invocation: CredentialInvocation,
    operation: &str,
    params: Value,
) -> Result<Value, CommandError> {
    if operation == "prepare" {
        return serde_json::to_value(prepare(ctx, invocation, decode(params)?).await?)
            .map_err(|_| invalid());
    }
    if operation == "commit" {
        #[derive(Deserialize)]
        struct Commit {
            ticket: Uuid,
            values: Vec<CapturedCredential>,
            result: Value,
        }
        let p: Commit = decode(params)?;
        commit(ctx, invocation, p.ticket, p.values, p.result).await?;
        return Ok(Value::Null);
    }
    #[derive(Deserialize)]
    struct Read {
        usage: CredentialUsage,
        #[serde(default)]
        names: Vec<String>,
    }
    let request: Read = decode(params)?;
    if request.names.len() > 32 || request.names.iter().any(|n| !validate_name(n)) {
        return Err(invalid());
    }
    let actor = access::actor(ctx, invocation, request.usage).await?;
    let rows = available(ctx, &actor).await?;
    match operation {
        "list" => Ok(json!(
            rows.iter()
                .map(|(row, record)| info(row, record))
                .collect::<Result<Vec<_>, _>>()?
        )),
        "resolve" => {
            let encryption = ctx
                .encryption
                .as_deref()
                .ok_or_else(|| CommandError::unavailable("Credential encryption is unavailable"))?;
            let mut result = Vec::new();
            for name in request.names {
                let candidates: Vec<_> = rows
                    .iter()
                    .filter(|(r, _)| r.env_name.as_deref() == Some(name.as_str()))
                    .collect();
                if candidates.len() != 1 {
                    return Err(CommandError::bad_request(format!(
                        "Credential ${name} is unavailable or ambiguous; reconnect or select a unique binding"
                    )));
                }
                let (row, record) = candidates[0];
                if row.expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
                    return Err(CommandError::bad_request(format!(
                        "Credential ${name} expired; renew or reconnect"
                    )));
                }
                let value = if let Some(connection_id) = row.source_connection_id {
                    resolve_connection(
                        ctx,
                        invocation,
                        &actor,
                        connection_id,
                        row.source_github_access.clone().ok_or_else(invalid)?,
                    )
                    .await?
                } else {
                    encryption
                        .decrypt_to_string(row.value_encrypted.as_deref().ok_or_else(|| {
                            CommandError::bad_request("Credential needs secure setup")
                        })?)
                        .map_err(|_| {
                            CommandError::unavailable("Credential could not be decrypted")
                        })?
                };
                sqlx::query("UPDATE credential_bindings SET last_used_at=now(),last_used_agent_id=$2 WHERE id=$1 AND org_id=$3")
        .bind(row.id)
        .bind(actor.agent_id)
        .bind(ctx.org_id())
        .execute(ctx.db.pool()).await
        .map_err(db_error)?;
                result.push(ResolvedCredential {
                    info: info(row, record)?,
                    value,
                });
            }
            Ok(json!(result))
        }
        _ => Err(invalid()),
    }
}

async fn resolve_connection(
    ctx: &Ctx,
    invocation: CredentialInvocation,
    actor: &ResourceActor,
    id: Uuid,
    request: Value,
) -> Result<String, CommandError> {
    let record = access::record(ctx, id).await?;
    if record.kind != "connection"
        || !access::permits_github(
            ctx,
            &record,
            actor,
            Some(&decode::<everruns_contracts::runtime::GitHubAccess>(
                request.clone(),
            )?),
        )
        .await?
    {
        return Err(CommandError::forbidden("Connection access denied"));
    }
    let request: everruns_contracts::runtime::GitHubAccess = decode(request)?;
    let constraint = ResourceConstraints::Github {
        repositories: request.repositories.clone(),
        permissions: request.permissions.clone(),
    };
    if actor.usage != CredentialUsage::Service
        || request.acts_as != everruns_core::McpServerActsAs::Service
        || !constraint.validate()
    {
        return Err(invalid());
    }
    // Only the native service connection or its exact shared org source may
    // answer. The reference cannot manufacture another acting identity.
    let selected:Option<Value>=sqlx::query_scalar(
            r#"
            SELECT jsonb_build_object('id',c.id,'source',c.provider_metadata->>'org_connection_id') FROM virtual_user_connections c
            WHERE c.org_id=$1 AND c.virtual_user_id=$2 AND c.provider='github'
            "#,
        )
        .bind(ctx.org_id())
        .bind(actor.service_user_id)
        .fetch_optional(ctx.db.pool()).await
        .map_err(db_error)?;
    let id_string = id.to_string();
    if selected.is_none_or(|v| {
        v["id"].as_str() != Some(&id_string) && v["source"].as_str() != Some(&id_string)
    }) {
        return Err(CommandError::forbidden(
            "Connection does not match the selected service identity",
        ));
    }
    let resolver = ctx
        .connection_resolver
        .as_ref()
        .and_then(|r| r.for_execution(invocation.input_message_id))
        .ok_or_else(|| CommandError::unavailable("Provider credential adapter unavailable"))?;
    let credential = resolver
        .get_github_token(invocation.session_id, &request)
        .await
        .map_err(|_| CommandError::unavailable("Provider credential resolution failed; reconnect"))?
        .ok_or_else(|| CommandError::forbidden("Provider access denied; reconnect"))?;
    if credential.github_installation_scope.is_none() {
        return Err(CommandError::forbidden(
            "This connection cannot enforce repository and permission limits",
        ));
    }
    Ok(credential.token)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Target {
    name: String,
    scope: ResourceScope,
    owner_id: Option<Uuid>,
    subject_id: Option<Uuid>,
    usage: CredentialUsage,
    id: Uuid,
    revision: Option<i64>,
}

fn target(actor: &ResourceActor, spec: &CaptureSpec) -> Result<Target, CommandError> {
    let owner_id = match spec.scope {
        ResourceScope::Session => Some(actor.session_id),
        ResourceScope::Agent if actor.usage == CredentialUsage::Service => {
            Some(actor.agent_id.ok_or_else(invalid)?)
        }
        ResourceScope::VirtualUser => Some(actor.subject().ok_or_else(invalid)?),
        ResourceScope::Organization if actor.usage == CredentialUsage::Service => None,
        _ => {
            return Err(CommandError::forbidden(
                "Scope does not match the acting identity",
            ));
        }
    };
    Ok(Target {
        name: spec.name.clone(),
        scope: spec.scope,
        owner_id,
        subject_id: if spec.scope == ResourceScope::Session {
            actor.subject()
        } else {
            None
        },
        usage: actor.usage,
        id: Uuid::now_v7(),
        revision: None,
    })
}

pub fn owner_key(
    scope: ResourceScope,
    owner_id: Option<Uuid>,
    subject: Option<Uuid>,
    usage: CredentialUsage,
    org: i64,
) -> String {
    format!(
        "{}:{}:{}:{}",
        access::wire(scope),
        owner_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| org.to_string()),
        access::wire(usage),
        subject.map(|v| v.to_string()).unwrap_or_default()
    )
}

fn validate_plan(plan: &CapturePlan) -> Result<(), CommandError> {
    let source = url::Url::parse(&plan.source_url).map_err(|_| invalid())?;
    if source.scheme() != "https"
        || !source.username().is_empty()
        || source.password().is_some()
        || source.fragment().is_some()
        || plan.captures.is_empty()
        || plan.captures.len() > 8
        || serde_json::to_vec(&plan.arguments)
            .map_err(|_| invalid())?
            .len()
            > 256 * 1024
    {
        return Err(invalid());
    }
    let mut names = std::collections::HashSet::new();
    for spec in &plan.captures {
        if !validate_name(&spec.name)
            || !names.insert(&spec.name)
            || spec.binding.validate().is_err()
            || spec.expires_in_from.is_some() && spec.expires_at_from.is_some()
        {
            return Err(invalid());
        }
        for pointer in std::iter::once(&spec.from)
            .chain(spec.expires_in_from.iter())
            .chain(spec.expires_at_from.iter())
        {
            if !pointer.starts_with('/') || pointer.len() > 1024 || pointer.split('/').count() > 18
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

async fn prepare(
    ctx: &Ctx,
    invocation: CredentialInvocation,
    plan: CapturePlan,
) -> Result<CapturePreparation, CommandError> {
    validate_plan(&plan)?;
    let actor = access::actor(ctx, invocation, plan.usage).await?;
    let mut canonical = serde_json::to_value(&plan).map_err(|_| invalid())?;
    canonical.sort_all_objects();
    let serialized = serde_json::to_vec(&canonical).map_err(|_| invalid())?;
    let hash = hex::encode(Sha256::digest(&serialized));
    let mut tx = ctx.db.pool().begin().await.map_err(db_error)?;
    #[derive(sqlx::FromRow)]
    struct Pending {
        id: Uuid,
        status: String,
        expires_at: DateTime<Utc>,
        result: Option<Value>,
        targets: Value,
    }
    let previous: Option<Pending> = sqlx::query_as(
        r#"
            SELECT id,status,expires_at,result,targets FROM credential_capture_requests
            WHERE org_id=$1 AND session_id=$2 AND input_message_id=$3 AND plan_hash=$4
            FOR UPDATE
            "#,
    )
    .bind(ctx.org_id())
    .bind(invocation.session_id)
    .bind(invocation.input_message_id)
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?;
    if let Some(p) = previous {
        if p.status == "completed" {
            return Ok(CapturePreparation::Completed {
                result: p.result.ok_or_else(invalid)?,
            });
        }
        if p.expires_at <= Utc::now() || p.status == "declined" {
            return Err(CommandError::forbidden(
                "Credential consent expired or was declined",
            ));
        }
        let mut authorizer = None;
        if p.status == "pending" {
            use everruns_contracts::tool_approval_types::{
                StoredToolApproval, one_off_decision_storage_key,
            };
            let fingerprint = format!("credential:{}", p.id);
            let decision: Option<String> = sqlx::query_scalar(
                "DELETE FROM session_key_values WHERE session_id=$1 AND key=$2 RETURNING value",
            )
            .bind(invocation.session_id)
            .bind(one_off_decision_storage_key(&fingerprint))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?;
            let Some(decision) = decision else {
                return Ok(CapturePreparation::ConsentRequired {
                    ticket: p.id,
                    expires_at: p.expires_at,
                    summary: consent_summary(
                        ctx,
                        &plan,
                        &decode::<Vec<Target>>(p.targets.clone())?,
                    )
                    .await?,
                });
            };
            let decision: StoredToolApproval =
                serde_json::from_str(&decision).map_err(|_| invalid())?;
            if decision.tool != "web_fetch"
                || decision.fingerprint.as_deref() != Some(fingerprint.as_str())
                || decision.expires_at.is_none_or(|e| e <= Utc::now())
            {
                return Err(invalid());
            }
            authorizer = decision.authorizer_user_id;
            if !decision.allow {
                sqlx::query("UPDATE credential_capture_requests SET status='declined' WHERE id=$1")
                    .bind(p.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_error)?;
                tx.commit().await.map_err(db_error)?;
                return Err(CommandError::forbidden("Credential capture was declined"));
            }
        } else if p.status != "approved" {
            return Err(CommandError::bad_request(
                "This authentication request was already dispatched; its outcome is uncertain. Do not replay it automatically",
            ));
        }
        // Consent cannot survive rotation, revocation, or an owner change.
        let targets: Vec<Target> = decode(p.targets.clone())?;
        for (spec, expected) in plan.captures.iter().zip(&targets) {
            let current = target(&actor, spec)?;
            if current.owner_id != expected.owner_id
                || current.subject_id != expected.subject_id
                || current.usage != expected.usage
            {
                return Err(CommandError::forbidden(
                    "Credential owner changed; request fresh consent",
                ));
            }
            let revision:Option<i64>=sqlx::query_scalar("SELECT revision FROM resource_access WHERE id=$1 AND org_id=$2 AND revoked_at IS NULL FOR UPDATE")
        .bind(expected.id)
        .bind(ctx.org_id())
        .fetch_optional(&mut *tx).await
        .map_err(db_error)?;
            if revision != expected.revision {
                return Err(CommandError::forbidden(
                    "Credential changed; request fresh consent",
                ));
            }
            if expected.revision.is_none() {
                let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM credential_bindings WHERE org_id=$1 AND owner_key=$2 AND env_name=$3)")
        .bind(ctx.org_id())
        .bind(owner_key(expected.scope,expected.owner_id,expected.subject_id,expected.usage,ctx.org_id()))
        .bind(&expected.name)
        .fetch_one(&mut *tx).await
        .map_err(db_error)?;
                if occupied {
                    return Err(CommandError::forbidden(
                        "Credential target changed; request fresh consent",
                    ));
                }
            }
        }
        sqlx::query(
            r#"
            UPDATE credential_capture_requests
            SET status='executing',authorizer_user_id=coalesce($2,authorizer_user_id)
            WHERE id=$1
            "#,
        )
        .bind(p.id)
        .bind(authorizer)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        return Ok(CapturePreparation::Ready { ticket: p.id });
    }
    let mut targets = Vec::new();
    let mut consent = false;
    for spec in &plan.captures {
        let mut t = target(&actor, spec)?;
        let key = owner_key(t.scope, t.owner_id, t.subject_id, t.usage, actor.org_id);
        let existing: Option<(Uuid, i64)> = sqlx::query_as(
            r#"
            SELECT c.id,r.revision FROM credential_bindings c
            JOIN resource_access r ON r.id=c.id
            WHERE c.org_id=$1 AND c.owner_key=$2 AND c.env_name=$3 AND c.kind='http'
            "#,
        )
        .bind(ctx.org_id())
        .bind(key)
        .bind(&spec.name)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
        if let Some((id, revision)) = existing {
            t.id = id;
            t.revision = Some(revision);
            consent = true;
        }
        consent |= spec.scope != ResourceScope::Session
            || !same_origin(&plan.source_url, &spec.binding.url);
        targets.push(t);
    }
    let id = Uuid::now_v7();
    let expiry = Utc::now() + chrono::Duration::minutes(15);
    sqlx::query(
            r#"
            INSERT INTO credential_capture_requests(id,org_id,session_id,input_message_id,plan_hash,plan,targets,status,expires_at)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
            "#,
        )
        .bind(id)
        .bind(ctx.org_id())
        .bind(invocation.session_id)
        .bind(invocation.input_message_id)
        .bind(hash)
        .bind(json!(plan))
        .bind(json!(targets))
        .bind(if consent {"pending"}else{"executing"})
        .bind(expiry)
        .execute(&mut *tx).await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(if consent {
        CapturePreparation::ConsentRequired {
            ticket: id,
            expires_at: expiry,
            summary: consent_summary(ctx, &plan, &targets).await?,
        }
    } else {
        CapturePreparation::Ready { ticket: id }
    })
}

async fn consent_summary(
    ctx: &Ctx,
    plan: &CapturePlan,
    targets: &[Target],
) -> Result<Value, CommandError> {
    let mut credentials = Vec::new();
    for (spec, target) in plan.captures.iter().zip(targets) {
        let owner_label = match target.scope {
            ResourceScope::Organization => "This organization".to_string(),
            ResourceScope::Session => "This session".to_string(),
            ResourceScope::Agent => ctx
                .db
                .get_agent(
                    ctx.org_id(),
                    everruns_contracts::typed_id::AgentId::from_uuid(
                        target.owner_id.ok_or_else(invalid)?,
                    ),
                )
                .await?
                .map(|a| a.name)
                .ok_or_else(invalid)?,
            ResourceScope::VirtualUser => ctx
                .db
                .get_virtual_user(
                    ctx.org_id(),
                    everruns_contracts::typed_id::VirtualUserId::from_uuid(
                        target.owner_id.ok_or_else(invalid)?,
                    ),
                )
                .await?
                .map(|v| v.name)
                .ok_or_else(invalid)?,
        };
        let mut item = json!(spec);
        item["owner_label"] = json!(owner_label);
        item["replacing"] = json!(target.revision.is_some());
        credentials.push(item);
    }
    Ok(
        json!({"source":plan.source_url,"method":plan.arguments.get("method").cloned().unwrap_or(json!("GET")),"acting_as":plan.usage,"request":plan.arguments,"credentials":credentials}),
    )
}

async fn commit(
    ctx: &Ctx,
    invocation: CredentialInvocation,
    ticket: Uuid,
    values: Vec<CapturedCredential>,
    result: Value,
) -> Result<(), CommandError> {
    if values.len() > 8 || serde_json::to_vec(&result).map_err(|_| invalid())?.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let mut tx = ctx.db.pool().begin().await.map_err(db_error)?;
    let row:Option<(Value,Value)>=sqlx::query_as(
            r#"
            SELECT plan,targets FROM credential_capture_requests
            WHERE id=$1 AND org_id=$2 AND session_id=$3 AND input_message_id=$4 AND status='executing' AND expires_at>now()
            FOR UPDATE
            "#,
        )
        .bind(ticket)
        .bind(ctx.org_id())
        .bind(invocation.session_id)
        .bind(invocation.input_message_id)
        .fetch_optional(&mut *tx).await
        .map_err(db_error)?;
    let (plan, targets) =
        row.ok_or_else(|| CommandError::forbidden("Credential capture is no longer authorized"))?;
    let plan: CapturePlan = decode(plan)?;
    let targets: Vec<Target> = decode(targets)?;
    let actor = access::actor(ctx, invocation, plan.usage).await?;
    if values.is_empty() {
        let code = result
            .get("error_code")
            .and_then(Value::as_str)
            .filter(|s| {
                matches!(
                    *s,
                    "authorization_pending"
                        | "slow_down"
                        | "access_denied"
                        | "expired_token"
                        | "invalid_grant"
                        | "invalid_client"
                )
            })
            .ok_or_else(invalid)?;
        let retry = matches!(code, "authorization_pending" | "slow_down");
        sqlx::query("UPDATE credential_capture_requests SET status=$2,result=$3 WHERE id=$1")
            .bind(ticket)
            .bind(if retry { "approved" } else { "completed" })
            .bind(if retry { None } else { Some(result) })
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        return Ok(());
    }
    if values.len() != targets.len() {
        return Err(invalid());
    }
    let encryption = ctx
        .encryption
        .as_deref()
        .ok_or_else(|| CommandError::unavailable("Credential encryption unavailable"))?;
    for ((spec, t), value) in plan.captures.iter().zip(targets.iter()).zip(values.iter()) {
        let expected = target(&actor, spec)?;
        if expected.owner_id != t.owner_id
            || expected.subject_id != t.subject_id
            || value.name != t.name
            || value.value.is_empty()
            || value.value.len() > 64 * 1024
            || value.expires_at.is_some_and(|e| e <= Utc::now())
        {
            return Err(invalid());
        }
        let encrypted = encryption
            .encrypt_string(&value.value)
            .map_err(|_| CommandError::unavailable("Credential encryption failed"))?;
        if let Some(revision) = t.revision {
            let live:Option<i64>=sqlx::query_scalar("SELECT revision FROM resource_access WHERE id=$1 AND org_id=$2 AND revoked_at IS NULL FOR UPDATE")
                .bind(t.id).bind(ctx.org_id()).fetch_optional(&mut *tx).await.map_err(db_error)?;
            if live != Some(revision) {
                return Err(CommandError::forbidden(
                    "Credential changed or was revoked; request fresh consent",
                ));
            }
            sqlx::query(
            r#"
            UPDATE credential_bindings
            SET binding=$2,value_encrypted=$3,source_connection_id=NULL,expires_at=$4,updated_at=now()
            WHERE id=$1
            "#,
        )
                .bind(t.id).bind(json!(spec.binding)).bind(encrypted).bind(value.expires_at).execute(&mut *tx).await.map_err(db_error)?;
        } else {
            insert_access(&mut tx, ctx.org_id(), t).await?;
            sqlx::query(
            r#"
            INSERT INTO credential_bindings(id,org_id,agent_id,kind,env_name,binding,label,value_encrypted,expires_at)
            VALUES($1,$2,$3,'http',$4,$5,$4,$6,$7)
            "#,
        )
        .bind(t.id)
        .bind(ctx.org_id())
        .bind(if t.scope==ResourceScope::Agent {t.owner_id}else{None})
        .bind(&t.name)
        .bind(json!(spec.binding))
        .bind(encrypted)
        .bind(value.expires_at)
        .execute(&mut *tx).await
        .map_err(db_error)?;
        }
    }
    for t in &targets {
        access::audit(
            ctx,
            &mut tx,
            t.id,
            "captured",
            Some(invocation.input_message_id),
        )
        .await?;
    }
    sqlx::query("UPDATE credential_capture_requests SET status='completed',result=$2 WHERE id=$1")
        .bind(ticket)
        .bind(result)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}

async fn insert_access(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: i64,
    t: &Target,
) -> Result<(), CommandError> {
    sqlx::query(
            r#"
            INSERT INTO resource_access(id,org_id,kind,scope,owner_agent_id,owner_session_id,owner_virtual_user_id,subject_id,usage)
            VALUES($1,$2,'credential_binding',$3,$4,$5,$6,$7,$8)
            "#,
        )
        .bind(t.id).bind(org).bind(access::wire(t.scope)).bind(if t.scope==ResourceScope::Agent {t.owner_id}else{None})
        .bind(if t.scope==ResourceScope::Session {t.owner_id}else{None}).bind(if t.scope==ResourceScope::VirtualUser {t.owner_id}else{None})
        .bind(t.subject_id).bind(access::wire(t.usage)).execute(&mut **tx).await.map_err(db_error)?;
    Ok(())
}

/// Called only by the authenticated durable-approval answer path.
pub async fn authorize_capture_answer(
    ctx: &Ctx,
    session: everruns_contracts::typed_id::SessionId,
    id: Uuid,
) -> Result<(), CommandError> {
    if ctx.caller.read_only {
        return Err(CommandError::forbidden(
            "Read-only credentials cannot approve credential storage",
        ));
    }
    let row: Option<(Value, Value)> = sqlx::query_as(
        r#"
            SELECT plan,targets FROM credential_capture_requests
            WHERE id=$1 AND org_id=$2 AND session_id=$3 AND status='pending' AND expires_at>now()
            "#,
    )
    .bind(id)
    .bind(ctx.org_id())
    .bind(session)
    .fetch_optional(ctx.db.pool())
    .await
    .map_err(db_error)?;
    let (plan, targets) = row.ok_or_else(|| {
        CommandError::forbidden("Credential request expired or was already answered")
    })?;
    let plan: CapturePlan = decode(plan)?;
    let targets: Vec<Target> = decode(targets)?;
    for t in &targets {
        let resource = AccessRecord {
            id: t.id,
            org_id: ctx.org_id(),
            kind: "credential_binding".into(),
            scope: access::wire(t.scope),
            owner_agent_id: if t.scope == ResourceScope::Agent {
                t.owner_id
            } else {
                None
            },
            owner_session_id: if t.scope == ResourceScope::Session {
                t.owner_id
            } else {
                None
            },
            owner_virtual_user_id: if t.scope == ResourceScope::VirtualUser {
                t.owner_id
            } else {
                None
            },
            subject_id: t.subject_id,
            usage: access::wire(plan.usage),
            revision: 1,
            revoked_at: None,
        };
        access::require_manage(ctx, &resource).await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
pub(crate) struct McpCredentialRow {
    #[sqlx(default)]
    pub setup_url: String,
    id: Uuid,
    pub mcp_server_name: String,
    pub mcp_server_url: String,
    pub tool_name: String,
    pub parameter_name: String,
    pub label: String,
    pub value_encrypted: Option<Vec<u8>>,
    pub expires_at: Option<DateTime<Utc>>,
}
pub(crate) async fn mcp_rows(
    ctx: &Ctx,
    invocation: CredentialInvocation,
    acts_as: everruns_core::McpServerActsAs,
) -> Result<Vec<McpCredentialRow>, CommandError> {
    use everruns_core::McpServerActsAs;
    let usages: &[CredentialUsage] = match acts_as {
        McpServerActsAs::None => &[],
        McpServerActsAs::Service => &[CredentialUsage::Service],
        McpServerActsAs::User => &[CredentialUsage::User],
        McpServerActsAs::UserOrService => &[CredentialUsage::User, CredentialUsage::Service],
    };
    let mut result = Vec::new();
    for usage in usages {
        let actor = match access::actor(ctx, invocation, *usage).await {
            Ok(a) => a,
            Err(e)
                if *usage == CredentialUsage::User && acts_as == McpServerActsAs::UserOrService =>
            {
                let _ = e;
                continue;
            }
            Err(e) => return Err(e),
        };
        let rows:Vec<McpCredentialRow>=sqlx::query_as(
            r#"
            SELECT c.id,c.mcp_server_name,c.mcp_server_url,c.tool_name,c.parameter_name,c.label,c.value_encrypted,c.expires_at FROM credential_bindings c
            JOIN resource_access r ON r.id=c.id AND r.org_id=c.org_id
            WHERE c.org_id=$1 AND c.kind='mcp_parameter' AND r.revoked_at IS NULL AND (r.owner_agent_id=$2 OR r.owner_virtual_user_id=$3 OR r.owner_session_id=$4 OR EXISTS(SELECT 1 FROM resource_grants g
            WHERE g.resource_id=r.id AND g.org_id=r.org_id AND g.revoked_at IS NULL AND (g.consumer_agent_id=$2 OR g.consumer_virtual_user_id=$3 OR g.consumer_session_id=$4 OR g.consumer_scope='organization')))
            ORDER BY c.id
            LIMIT 257
            "#,
        )
        .bind(ctx.org_id())
        .bind(actor.agent_id)
        .bind(actor.subject())
        .bind(actor.session_id)
        .fetch_all(ctx.db.pool()).await
        .map_err(db_error)?;
        if rows.len() > 256 {
            return Err(invalid());
        }
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        let allowed = access::authorized_records(ctx, &ids, &actor).await?;
        for mut row in rows {
            if let Some(record) = allowed.get(&row.id)
                && !result.iter().any(|v: &McpCredentialRow| v.id == row.id)
            {
                row.setup_url = match access::scope(&record.scope)? {
                    ResourceScope::Organization => "/settings/organization".into(),
                    ResourceScope::Session => format!(
                        "/sessions/{}",
                        everruns_contracts::typed_id::SessionId::from_uuid(
                            record.owner_session_id.ok_or_else(invalid)?
                        )
                    ),
                    ResourceScope::VirtualUser => {
                        if record.usage == "user" {
                            "/settings/agent-experience".into()
                        } else {
                            format!(
                                "/virtual-users/{}?tab=connections",
                                everruns_contracts::typed_id::VirtualUserId::from_uuid(
                                    record.owner_virtual_user_id.ok_or_else(invalid)?
                                )
                            )
                        }
                    }
                    ResourceScope::Agent => {
                        let id = everruns_contracts::typed_id::AgentId::from_uuid(
                            record.owner_agent_id.ok_or_else(invalid)?,
                        );
                        let public = ctx
                            .db
                            .get_agent_public_id(ctx.org_id(), id)
                            .await?
                            .unwrap_or_else(|| id.to_string());
                        format!("/agents/{public}?tab=credentials")
                    }
                };
                result.push(row);
            }
        }
    }
    Ok(result)
}
