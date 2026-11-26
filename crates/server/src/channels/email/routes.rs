//! Admin control plane and a signature-authenticated, durable webhook endpoint.
//! Provider keys and webhook secrets never appear in response DTOs.
use super::{
    compose,
    queue::{Batch, Disposition, EmailQueue, MailSummary},
};
use crate::api::common::ErrorResponse;
use crate::auth::middleware::{AuthState, ResolvedOrg};
use axum::extract::{FromRef, FromRequestParts};
struct EmailAdmin(ResolvedOrg);
impl<S> FromRequestParts<S> for EmailAdmin
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = crate::auth::AuthError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let org = ResolvedOrg::from_request_parts(parts, state).await?;
        if !org.role.has_permission(everruns_core::OrgRole::Admin)
            || (org.read_only
                && !matches!(
                    parts.method,
                    axum::http::Method::GET | axum::http::Method::HEAD
                ))
        {
            return Err(crate::auth::AuthError::forbidden(
                "Organization administrator with write access required",
            ));
        }
        Ok(Self(org))
    }
}
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

pub type AppState = crate::api::virtual_user_connections::AppState;
type ApiError = (StatusCode, Json<ErrorResponse>);
fn bad(message: &str) -> ApiError {
    ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST)
}
fn missing() -> ApiError {
    ErrorResponse::new("Inbox or message not found").into_response(StatusCode::NOT_FOUND)
}
fn internal(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error,"Email operation failed");
    ErrorResponse::new("Email operation failed").into_response(StatusCode::INTERNAL_SERVER_ERROR)
}
fn queue(state: &AppState) -> EmailQueue {
    EmailQueue::new(state.db.pool().clone())
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/platform/email/inboxes",
            get(list_inboxes).post(create_inbox),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/messages",
            get(list_messages),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/messages/{message}",
            get(read_message),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/batches/claim",
            post(claim),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/batches/{lease}/dispositions",
            post(dispose),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/outbox",
            get(list_outbox),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/outbox/{id}/approve",
            post(approve_reply),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/messages/{message}/approve",
            post(approve_execution),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}",
            axum::routing::patch(update_inbox),
        )
        .route(
            "/v1/platform/email/templates",
            get(list_templates).post(create_template),
        )
        .route(
            "/v1/platform/email/templates/preview",
            post(preview_template),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/messages/{message}/dismiss",
            post(dismiss_message),
        )
        .route(
            "/v1/platform/email/inboxes/{inbox}/outbox/{id}/reject",
            post(reject_reply),
        )
        .route("/v1/email/inboxes/{inbox}/webhook", post(webhook))
        .layer(DefaultBodyLimit::max(1_048_576))
        .with_state(state)
}

#[derive(Serialize, sqlx::FromRow)]
struct Inbox {
    id: Uuid,
    account_id: Uuid,
    provider: String,
    provider_inbox_id: String,
    address: String,
    name: String,
    enabled: bool,
    created_at: DateTime<Utc>,
}
#[utoipa::path(operation_id = "email_list_inboxes", get, path = "/v1/platform/email/inboxes", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")))]
async fn list_inboxes(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
) -> Result<Json<Vec<Inbox>>, ApiError> {
    Ok(Json(sqlx::query_as("SELECT id,account_id,provider,provider_inbox_id,address,name,enabled,created_at FROM email_inboxes WHERE org_id=$1 ORDER BY created_at,id LIMIT 100").bind(org.org_id).fetch_all(s.db.pool()).await.map_err(internal)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInbox {
    account_id: Uuid,
    address: String,
    name: String,
    signing_secret: Option<String>,
}
#[utoipa::path(operation_id = "email_create_inbox", post, path = "/v1/platform/email/inboxes", tag = "email", responses((status = 201, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), request_body = Value)]
async fn create_inbox(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Json(body): Json<CreateInbox>,
) -> Result<(StatusCode, Json<Inbox>), ApiError> {
    channelkit_agentmail::mailbox(&body.address).map_err(|_| bad("Invalid inbox address"))?;
    if body.name.trim().is_empty() || body.name.len() > 255 {
        return Err(bad("Name must be 1 to 255 bytes"));
    }
    let secret = body.signing_secret.filter(|v| !v.is_empty());
    if let Some(secret) = &secret {
        validate_secret(secret)?;
    }
    let connection = s
        .db
        .get_organization_connection(org.org_id, body.account_id)
        .await
        .map_err(internal)?
        .filter(|r| r.provider == "agentmail_inbox" && r.access_token_encrypted.is_some())
        .ok_or_else(|| bad("Select an AgentMail inbox provider account in this organization"))?;
    let encryption = s
        .encryption
        .as_ref()
        .ok_or_else(|| bad("Encryption must be configured"))?;
    let key = encryption
        .decrypt_to_string(
            connection
                .access_token_encrypted
                .as_ref()
                .expect("validated credential"),
        )
        .map_err(internal)?;
    let client = channelkit_agentmail::AgentMail::new(
        key,
        body.address.clone(),
        std::sync::Arc::new(channelkit::http::ReqwestTransport::new().map_err(internal)?),
    )
    .map_err(internal)?;
    client
        .verify_inbox()
        .await
        .map_err(|_| bad("The provider account cannot access this inbox"))?;
    let encrypted = encryption
        .encrypt_string(secret.as_deref().unwrap_or_default())
        .map_err(internal)?;
    let mut tx = s.db.pool().begin().await.map_err(internal)?;
    sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
        .bind(org.org_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM email_inboxes WHERE org_id=$1")
        .bind(org.org_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if count >= 100 {
        return Err(bad("Inbox limit reached"));
    }
    let row=sqlx::query_as("INSERT INTO email_inboxes (id,org_id,account_id,provider,provider_inbox_id,address,name,signing_secret_encrypted,enabled) VALUES ($1,$2,$3,'agentmail',$4,$4,$5,$6,$7) RETURNING id,account_id,provider,provider_inbox_id,address,name,enabled,created_at")
        .bind(Uuid::now_v7()).bind(org.org_id).bind(connection.id).bind(&body.address).bind(body.name.trim()).bind(encrypted).bind(secret.is_some()).fetch_one(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Default, Deserialize)]
struct ListQuery {
    before: Option<Uuid>,
}
#[utoipa::path(operation_id = "email_list_messages", get, path = "/v1/platform/email/inboxes/{inbox}/messages", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier"),("before" = Option<Uuid>, Query, description = "Exclusive message cursor")))]
async fn list_messages(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path(inbox): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<MailSummary>>, ApiError> {
    Ok(Json(
        queue(&s)
            .list(org.org_id, inbox, q.before)
            .await
            .map_err(internal)?,
    ))
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReadView {
    Preview,
    #[default]
    Clean,
    Full,
}
#[derive(Default, Deserialize)]
struct ReadQuery {
    #[serde(default)]
    view: ReadView,
    #[serde(default)]
    offset: usize,
}
#[utoipa::path(operation_id = "email_read_message", get, path = "/v1/platform/email/inboxes/{inbox}/messages/{message}", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier"),("message" = Uuid, Path, description = "Scoped resource identifier"),("view" = Option<String>, Query, description = "preview, clean or full"),("offset" = Option<usize>, Query, description = "UTF-8 byte offset")))]
async fn read_message(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path((inbox, id)): Path<(Uuid, Uuid)>,
    Query(q): Query<ReadQuery>,
) -> Result<Json<Value>, ApiError> {
    let mail = if matches!(q.view, ReadView::Preview) {
        queue(&s)
            .message(org.org_id, inbox, id)
            .await
            .map_err(internal)?
            .ok_or_else(missing)?
    } else {
        super::provider::hydrate(&s.db, s.encryption.as_ref(), org.org_id, inbox, id)
            .await
            .map_err(internal)?
    };
    let (content, source, omitted, warnings) = match q.view {
        ReadView::Preview => (
            mail.preview,
            "provider_preview".to_string(),
            json!([]),
            json!([]),
        ),
        ReadView::Clean => {
            let v = super::provider::clean(&mail).map_err(internal)?;
            (v.clean, v.source, json!(v.omitted), json!(v.warnings))
        }
        ReadView::Full => {
            let v = markkit::email::extract_body(
                markkit::email::EmailBody {
                    text: mail.text.as_deref(),
                    html: mail.html.as_deref(),
                },
                markkit::Limits::default()
                    .max_input_size(1_048_576)
                    .max_output_size(1_048_576),
            )
            .map_err(internal)?;
            (v.full, v.source, json!([]), json!(v.warnings))
        }
    };
    if q.offset > content.len() || !content.is_char_boundary(q.offset) {
        return Err(bad("Invalid content offset"));
    }
    let mut end = q.offset.saturating_add(16_384).min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    Ok(Json(
        json!({"id":id,"content":&content[q.offset..end],"source":source,"untrusted":true,"omitted":omitted,"warnings":warnings,"next_offset":(end<content.len()).then_some(end),"attachments":mail.attachments}),
    ))
}
#[utoipa::path(operation_id = "email_claim", post, path = "/v1/platform/email/inboxes/{inbox}/batches/claim", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier")))]
async fn claim(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path(inbox): Path<Uuid>,
) -> Result<Json<Option<Batch>>, ApiError> {
    Ok(Json(
        queue(&s).claim(org.org_id, inbox).await.map_err(internal)?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DispositionInput {
    id: Uuid,
    disposition: Disposition,
    reason: String,
}
#[utoipa::path(operation_id = "email_dispose", post, path = "/v1/platform/email/inboxes/{inbox}/batches/{lease}/dispositions", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier"),("lease" = Uuid, Path, description = "Scoped resource identifier")), request_body = Value)]
async fn dispose(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path((inbox, lease)): Path<(Uuid, Uuid)>,
    Json(input): Json<Vec<DispositionInput>>,
) -> Result<Json<Value>, ApiError> {
    if input.len() > 25 {
        return Err(bad("At most 25 dispositions"));
    }
    let mut result = Vec::new();
    for i in input {
        result.push(json!({"id":i.id,"applied":queue(&s).dispose(org.org_id,inbox,i.id,lease,i.disposition,&i.reason).await.map_err(internal)?}));
    }
    Ok(Json(json!(result)))
}

#[utoipa::path(operation_id = "email_webhook", post, path = "/v1/email/inboxes/{inbox}/webhook", tag = "email", request_body(content=String,content_type="application/json"), responses((status = 202, description = "Receipt durably persisted or already known"), (status = 401, description = "Invalid webhook signature"), (status = 503, description = "Intake capacity unavailable; retry delivery")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier")))]
async fn webhook(
    State(s): State<AppState>,
    Path(inbox): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let row:Option<(i64,String,Vec<u8>)>=sqlx::query_as("SELECT org_id,provider_inbox_id,signing_secret_encrypted FROM email_inboxes WHERE id=$1 AND enabled").bind(inbox).fetch_optional(s.db.pool()).await.map_err(internal)?;
    let (org, provider_inbox, secret) = row.ok_or_else(missing)?;
    let secret = s
        .encryption
        .as_ref()
        .ok_or_else(|| internal("missing encryption"))?
        .decrypt_to_string(&secret)
        .map_err(internal)?;
    let mut req = channelkit::WebhookRequest::post(body.to_vec());
    for name in ["svix-id", "svix-timestamp", "svix-signature"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            req = req.header(name, v);
        }
    }
    let event = channelkit_agentmail::verify_received(
        &req,
        &secret,
        &provider_inbox,
        Utc::now().timestamp(),
    )
    .map_err(|_| {
        ErrorResponse::new("Webhook verification failed").into_response(StatusCode::UNAUTHORIZED)
    })?;
    if let Some(event) = event {
        queue(&s)
            .ingest(org, inbox, &event)
            .await
            .map_err(|error| {
                tracing::warn!(%inbox,%error,"Email intake refused");
                ErrorResponse::new("Email intake temporarily unavailable")
                    .into_response(StatusCode::SERVICE_UNAVAILABLE)
            })?;
    }
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(operation_id = "email_list_templates", get, path = "/v1/platform/email/templates", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")))]
async fn list_templates(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
) -> Result<Json<Vec<Value>>, ApiError> {
    Ok(Json(sqlx::query_scalar("SELECT to_jsonb(t) - 'org_id' FROM (SELECT DISTINCT ON (id) * FROM email_templates WHERE org_id=$1 ORDER BY id,version DESC) t LIMIT 100").bind(org.org_id).fetch_all(s.db.pool()).await.map_err(internal)?))
}
#[derive(Deserialize)]
struct TemplateInput {
    id: Option<Uuid>,
    #[serde(flatten)]
    template: compose::EmailTemplate,
}
#[utoipa::path(operation_id = "email_create_template", post, path = "/v1/platform/email/templates", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), request_body = Value)]
async fn create_template(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Json(input): Json<TemplateInput>,
) -> Result<Json<Value>, ApiError> {
    let t = input.template;
    if t.name.trim().is_empty()
        || t.name.len() > 255
        || t.subject.len() > 998
        || t.html.as_ref().map_or(0, String::len) + t.text.as_ref().map_or(0, String::len) > 262_144
        || (t.html.is_none() && t.text.is_none())
    {
        return Err(bad("Invalid template"));
    }
    let id = input.id.unwrap_or_else(Uuid::now_v7);
    let mut tx = s.db.pool().begin().await.map_err(internal)?;
    sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
        .bind(org.org_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM email_templates WHERE org_id=$1")
        .bind(org.org_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if count >= 1000 {
        return Err(bad("Template version limit reached"));
    }
    let version: i32 = sqlx::query_scalar(
        "SELECT coalesce(max(version),0)+1 FROM email_templates WHERE org_id=$1 AND id=$2",
    )
    .bind(org.org_id)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query("INSERT INTO email_templates (id,org_id,version,name,subject,html,text_body) VALUES ($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(org.org_id).bind(version).bind(t.name).bind(t.subject).bind(t.html).bind(t.text).execute(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"id":id,"version":version})))
}
#[derive(Deserialize)]
struct PreviewInput {
    template: compose::EmailTemplate,
    variables: std::collections::BTreeMap<String, String>,
}
#[utoipa::path(operation_id = "email_preview_template", post, path = "/v1/platform/email/templates/preview", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), request_body = Value)]
async fn preview_template(
    EmailAdmin(_org): EmailAdmin,
    Json(input): Json<PreviewInput>,
) -> Result<Json<compose::ComposedEmail>, ApiError> {
    compose::render(&input.template, &input.variables)
        .map(Json)
        .map_err(|e| bad(&e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InboxUpdate {
    enabled: Option<bool>,
    signing_secret: Option<String>,
}
#[utoipa::path(operation_id = "email_update_inbox", patch, path = "/v1/platform/email/inboxes/{inbox}", tag = "email", responses((status = 204, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier")), request_body = Value)]
async fn update_inbox(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path(inbox): Path<Uuid>,
    Json(body): Json<InboxUpdate>,
) -> Result<StatusCode, ApiError> {
    let (previous, enabled): (Vec<u8>, bool) = sqlx::query_as(
        "SELECT signing_secret_encrypted,enabled FROM email_inboxes WHERE org_id=$1 AND id=$2",
    )
    .bind(org.org_id)
    .bind(inbox)
    .fetch_optional(s.db.pool())
    .await
    .map_err(internal)?
    .ok_or_else(missing)?;
    let encryption = s
        .encryption
        .as_ref()
        .ok_or_else(|| bad("Encryption must be configured"))?;
    let encrypted = if let Some(secret) = body.signing_secret {
        validate_secret(&secret)?;
        encryption.encrypt_string(&secret).map_err(internal)?
    } else {
        previous
    };
    let enabled = body.enabled.unwrap_or(enabled);
    if enabled {
        validate_secret(&encryption.decrypt_to_string(&encrypted).map_err(internal)?)?;
    }
    sqlx::query(
        "UPDATE email_inboxes SET enabled=$3,signing_secret_encrypted=$4 WHERE org_id=$1 AND id=$2",
    )
    .bind(org.org_id)
    .bind(inbox)
    .bind(enabled)
    .bind(encrypted)
    .execute(s.db.pool())
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(operation_id = "email_list_outbox", get, path = "/v1/platform/email/inboxes/{inbox}/outbox", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier")))]
async fn list_outbox(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path(inbox): Path<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    Ok(Json(sqlx::query_scalar("SELECT jsonb_build_object('id',id,'intake_id',intake_id,'session_id',session_id,'body',body,'state',state,'created_at',created_at) FROM email_outbox WHERE org_id=$1 AND inbox_id=$2 AND intake_id IN (SELECT id FROM email_intake WHERE archived_at IS NULL) ORDER BY created_at DESC LIMIT 100").bind(org.org_id).bind(inbox).fetch_all(s.db.pool()).await.map_err(internal)?))
}
#[utoipa::path(operation_id = "email_approve_reply", post, path = "/v1/platform/email/inboxes/{inbox}/outbox/{id}/approve", tag = "email", responses((status = 200, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier"),("id" = Uuid, Path, description = "Scoped resource identifier")))]
async fn approve_reply(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path((inbox, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let changed = sqlx::query("UPDATE email_outbox o SET authorized=true,approved_by=$4,updated_at=now() FROM email_intake m WHERE o.org_id=$1 AND o.inbox_id=$2 AND o.id=$3 AND o.state='held' AND m.id=o.intake_id AND m.state='completed'")
        .bind(org.org_id).bind(inbox).bind(id).bind(org.user_id).execute(s.db.pool()).await.map_err(internal)?;
    if changed.rows_affected() != 1 {
        return Err(bad("Only replies from completed turns can be approved"));
    }
    super::outbox::send(&s.db, s.encryption.as_ref(), org.org_id, inbox, id)
        .await
        .map(|state| Json(json!({"state":state})))
        .map_err(|_| bad("Email send could not be approved; inspect its current state"))
}
#[utoipa::path(operation_id = "email_approve_execution", post, path = "/v1/platform/email/inboxes/{inbox}/messages/{message}/approve", tag = "email", responses((status = 202, description = "Agent execution approved; sending requires separate approval"), (status = 401, description = "Authentication required"), (status = 403, description = "Organization administrator required")), params(("inbox" = Uuid, Path, description = "Scoped resource identifier"),("message" = Uuid, Path, description = "Scoped resource identifier")))]
async fn approve_execution(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path((inbox, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    // Review is explicit authority to run the configured agent. It never
    // changes sender authentication or grants permission to send its output.
    let mail = super::provider::hydrate(&s.db, s.encryption.as_ref(), org.org_id, inbox, id)
        .await
        .map_err(internal)?;
    super::provider::sender(&mail).map_err(|_| bad("A single sender mailbox is required"))?;
    let changed=sqlx::query("UPDATE email_intake SET state='ready',approved=true,approved_by=$4,reason='operator_approved_execution',available_at=now(),updated_at=now() WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND state='held'").bind(org.org_id).bind(inbox).bind(id).bind(org.user_id).execute(s.db.pool()).await.map_err(internal)?;
    if changed.rows_affected() != 1 {
        return Err(bad("Only held messages may be approved"));
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    list_inboxes,
    create_inbox,
    list_messages,
    read_message,
    claim,
    dispose,
    webhook,
    list_templates,
    create_template,
    preview_template,
    update_inbox,
    list_outbox,
    approve_reply,
    approve_execution,
    dismiss_message,
    reject_reply
))]
pub struct EmailApi;

fn validate_secret(secret: &str) -> Result<(), ApiError> {
    if secret
        .strip_prefix("whsec_")
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .is_some_and(|v| (16..=256).contains(&v.len()))
    {
        Ok(())
    } else {
        Err(bad("Invalid webhook signing secret"))
    }
}

#[utoipa::path(operation_id = "email_dismiss_message", post,path="/v1/platform/email/inboxes/{inbox}/messages/{message}/dismiss",tag="email",params(("inbox"=Uuid,Path),("message"=Uuid,Path)),responses((status=204,description="Message dismissed")))]
async fn dismiss_message(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path((inbox, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let changed=sqlx::query("UPDATE email_intake SET state='ignored',reason='operator_dismissed',approved_by=$4,updated_at=now() WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND state IN ('held','pending','deferred')").bind(org.org_id).bind(inbox).bind(id).bind(org.user_id).execute(s.db.pool()).await.map_err(internal)?;
    if changed.rows_affected() != 1 {
        return Err(bad(
            "Only messages awaiting processing or review can be dismissed",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(operation_id = "email_reject_reply", post,path="/v1/platform/email/inboxes/{inbox}/outbox/{id}/reject",tag="email",params(("inbox"=Uuid,Path),("id"=Uuid,Path)),responses((status=204,description="Reply rejected")))]
async fn reject_reply(
    State(s): State<AppState>,
    EmailAdmin(org): EmailAdmin,
    Path((inbox, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let changed=sqlx::query("UPDATE email_outbox SET state='failed',authorized=false,gate_state='review',approved_by=$4,updated_at=now() WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND state='held'").bind(org.org_id).bind(inbox).bind(id).bind(org.user_id).execute(s.db.pool()).await.map_err(internal)?;
    if changed.rows_affected() != 1 {
        return Err(bad("Only held replies can be rejected"));
    }
    Ok(StatusCode::NO_CONTENT)
}
