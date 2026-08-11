// OSS-owned organization invitations (EVE-602).
//
// Everruns owns the full org membership lifecycle: creating invites, listing
// pending invites, revoking, validating tokens, and accepting. Membership never
// depends on an external identity provider's org claims.
//
// Email delivery is optional. `email_sender` is always present (the platform
// wires a `DisabledEmailSender` when no provider is configured), so creation
// attempts delivery and classifies the outcome:
//   - Ok                              -> EmailDelivery::Sent
//   - Err(EmailError::Configuration)  -> EmailDelivery::NotConfigured (disabled)
//   - Err(_)                          -> EmailDelivery::Failed (transport/provider)
// In every case the invite is persisted first and the raw token is returned once
// as an `invite_url`, so a failed/disabled delivery degrades to copy-link UX
// without losing the pending invite.
//
// SECURITY: only the SHA-256 token hash is stored; the raw token is never
// persisted or logged. Because the raw token cannot be recovered, the invite URL
// is only available at creation time — listing returns metadata without a URL.

use crate::auth::audit;
use crate::auth::middleware::{AuthState, AuthUser, OrgAdmin};
use crate::storage::StorageBackend;
use crate::storage::models::CreateOrgInvitation;
use axum::{
    Json, Router,
    extract::{ConnectInfo, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use everruns_core::OrgRole;
use everruns_platform::email::{EmailError, EmailMessage, EmailSender};
use everruns_platform::{AuditEvent, ManagementAction};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::{ApiResultExt, ErrorResponse, ListResponse, impl_auth_state};

/// Invite tokens live for 7 days by default.
const INVITE_TTL_DAYS: i64 = 7;
/// Token prefix; distinguishes invite tokens from PATs/JWTs.
const INVITE_TOKEN_PREFIX: &str = "evrinv_";
/// Random bytes in the token body (64 hex chars).
const INVITE_TOKEN_BYTES: usize = 32;

/// App state for organization invitation routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    /// Optional transactional email delivery. Always present; may be disabled.
    pub email_sender: Arc<dyn EmailSender>,
    /// Public app origin used to build invite links (UI origin).
    pub frontend_url: String,
    pub resource_limits: crate::server::ResourceLimitsConfig,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        email_sender: Arc<dyn EmailSender>,
        frontend_url: String,
    ) -> Self {
        Self {
            db,
            auth,
            email_sender,
            frontend_url,
            resource_limits: crate::server::ResourceLimitsConfig::from_env(),
        }
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs/{org}/invites",
            get(list_invites).post(create_invite),
        )
        .route(
            "/v1/orgs/{org}/invites/{invite_id}",
            axum::routing::delete(revoke_invite),
        )
        .route("/v1/invites/{token}/accept", post(accept_invite))
        .with_state(state)
}

// ============================================================================
// Token + id helpers
// ============================================================================

/// Generate a fresh invite token and its storage hash. The raw token is shown
/// once (embedded in the invite URL) and never persisted.
pub fn generate_invite_token() -> (String, String) {
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..INVITE_TOKEN_BYTES).map(|_| rng.random()).collect();
    let token = format!("{INVITE_TOKEN_PREFIX}{}", hex::encode(&random_bytes));
    let hash = hash_invite_token(&token);
    (token, hash)
}

/// SHA-256 hash of an invite token for storage/lookup.
pub fn hash_invite_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn generate_invite_public_id() -> String {
    format!("orginv_{}", Uuid::new_v4().simple())
}

/// Normalize an email for storage and comparison: trim + lowercase.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// Escape a string for safe interpolation into HTML text or a double-quoted
/// attribute value. Covers `& < > " '`.
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Cheap structural email check. Full RFC validation happens in the email layer
/// at send time; this only guards obviously invalid input at creation.
fn is_plausible_email(email: &str) -> bool {
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    parts.next().is_none()
        && !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !email.chars().any(|c| c.is_whitespace())
}

// ============================================================================
// Status + DTOs
// ============================================================================

/// Lifecycle status derived from an invitation row's timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvitationStatus {
    Pending,
    Expired,
    Accepted,
    Revoked,
}

impl InvitationStatus {
    fn of(row: &crate::storage::models::OrgInvitationRow) -> Self {
        if row.accepted_at.is_some() {
            Self::Accepted
        } else if row.revoked_at.is_some() {
            Self::Revoked
        } else if row.expires_at <= chrono::Utc::now() {
            Self::Expired
        } else {
            Self::Pending
        }
    }
}

/// Outcome of the optional email-delivery attempt at creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmailDelivery {
    /// Email service configured and delivery accepted.
    Sent,
    /// Email service configured but delivery failed; invite remains pending.
    Failed,
    /// No email service configured; copy the invite link instead.
    NotConfigured,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateInviteRequest {
    /// Email address to invite.
    #[schema(example = "teammate@example.com")]
    pub email: String,
    /// Role to grant on acceptance. Defaults to `member`.
    #[serde(default)]
    #[schema(example = "member")]
    pub role: Option<String>,
}

/// Invitation metadata returned by list/create.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvitationResponse {
    /// Stable public invite id (orginv_...).
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: InvitationStatus,
    /// User id that created the invite.
    pub invited_by: String,
    pub created_at: String,
    pub expires_at: String,
}

/// Create response: invitation metadata plus the one-time invite URL and the
/// email-delivery outcome.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateInviteResponse {
    #[serde(flatten)]
    pub invitation: InvitationResponse,
    /// One-time invite URL (contains the raw token). Surfaced only here.
    pub invite_url: String,
    pub email_delivery: EmailDelivery,
}

/// Accept response: where the caller now has membership.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AcceptInviteResponse {
    pub org_id: String,
    pub role: String,
}

fn to_invitation_response(row: &crate::storage::models::OrgInvitationRow) -> InvitationResponse {
    InvitationResponse {
        id: row.public_id.clone(),
        email: row.email.clone(),
        role: row.role.clone(),
        status: InvitationStatus::of(row),
        invited_by: row.invited_by.to_string(),
        created_at: row.created_at.to_rfc3339(),
        expires_at: row.expires_at.to_rfc3339(),
    }
}

// ============================================================================
// Typed errors (protocol-agnostic for unit tests; map to HTTP at the edge)
// ============================================================================

/// Invite operation error with a stable, UI-facing code and HTTP status. Codes
/// are distinct per failure mode so the UI can react without leaking token
/// secrets.
#[derive(Debug)]
pub struct InviteError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl InviteError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

impl From<InviteError> for (StatusCode, Json<ErrorResponse>) {
    fn from(e: InviteError) -> Self {
        ErrorResponse::new(e.message)
            .with_code(e.code)
            .into_response(e.status)
    }
}

fn internal(context: &str, err: anyhow::Error) -> InviteError {
    tracing::error!(error = %err, "org invitation: {context}");
    InviteError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Internal server error",
    )
}

// ============================================================================
// Core operations (testable without HTTP)
// ============================================================================

/// Build the one-time invite URL from the public app origin and raw token.
fn invite_url(frontend_url: &str, token: &str) -> String {
    format!("{}/invite/{token}", frontend_url.trim_end_matches('/'))
}

/// Attempt optional email delivery and classify the result. Never returns an
/// error: delivery problems are reported as a status so the invite stays
/// pending and copy-link UX still works.
async fn deliver_invite_email(
    email_sender: &dyn EmailSender,
    org_name: &str,
    to: &str,
    url: &str,
) -> EmailDelivery {
    let subject = format!("You're invited to join {org_name} on Everruns");
    // Plain-text body is not markup; interpolate raw.
    let text = format!(
        "You've been invited to join {org_name} on Everruns.\n\nAccept your invitation:\n{url}\n\nThis link expires in {INVITE_TTL_DAYS} days."
    );
    // HTML body: escape user-controlled values (org name) and the URL before
    // interpolation to prevent HTML/attribute injection in email clients.
    let org_name_html = html_escape(org_name);
    let url_html = html_escape(url);
    let html = format!(
        "<p>You've been invited to join <strong>{org_name_html}</strong> on Everruns.</p>\
         <p><a href=\"{url_html}\">Accept your invitation</a></p>\
         <p>This link expires in {INVITE_TTL_DAYS} days.</p>"
    );
    // Branded template: this is an external-facing invitation, so it carries
    // the Everruns logo, wordmark, and a link back to everruns.com.
    let message = EmailMessage::basic(to, subject, text, html);
    match email_sender.send_email(message).await {
        Ok(_) => EmailDelivery::Sent,
        // A disabled/unconfigured sender reports a configuration error.
        Err(EmailError::Configuration(_)) => EmailDelivery::NotConfigured,
        Err(err) => {
            tracing::warn!(error = %err, "invite email delivery failed; invite remains pending");
            EmailDelivery::Failed
        }
    }
}

#[derive(Debug)]
pub struct CreatedInvitation {
    pub row: crate::storage::models::OrgInvitationRow,
    pub invite_url: String,
    pub email_delivery: EmailDelivery,
}

/// Create a pending invite. Enforces role-escalation rules, dedup, and member
/// limits, persists the hashed token, then attempts optional email delivery.
#[allow(clippy::too_many_arguments)]
pub async fn create_invitation(
    db: &StorageBackend,
    email_sender: &dyn EmailSender,
    frontend_url: &str,
    org_id: i64,
    org_name: &str,
    caller_role: OrgRole,
    caller_user_id: Uuid,
    email_raw: &str,
    role_raw: Option<&str>,
    max_members: i64,
) -> Result<CreatedInvitation, InviteError> {
    let email = normalize_email(email_raw);
    if !is_plausible_email(&email) {
        return Err(InviteError::new(
            StatusCode::BAD_REQUEST,
            "invalid_email",
            "A valid email address is required",
        ));
    }

    let role: OrgRole = role_raw.unwrap_or("member").parse().map_err(|_| {
        InviteError::new(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "Invalid role. Must be 'owner', 'admin', or 'member'",
        )
    })?;

    // Role-escalation rule mirrors member management: only owners invite owners.
    if role == OrgRole::Owner && !caller_role.has_permission(OrgRole::Owner) {
        return Err(InviteError::new(
            StatusCode::FORBIDDEN,
            "insufficient_role",
            "Only owners can invite owners",
        ));
    }

    // Reject inviting someone who is already a member.
    if let Some(user) = db
        .get_user_by_email(&email)
        .await
        .map_err(|e| internal("lookup user by email", e))?
        && db
            .is_organization_member(org_id, user.id)
            .await
            .map_err(|e| internal("check membership", e))?
    {
        return Err(InviteError::new(
            StatusCode::CONFLICT,
            "already_member",
            "That user is already a member of this organization",
        ));
    }

    // At most one outstanding invite per (org, email), matching the partial
    // unique index. An expired-but-unrevoked row still occupies that slot, so we
    // supersede it (revoke) before reissuing — otherwise the INSERT below would
    // hit the unique index and surface as a 500 instead of allowing reissue.
    if let Some(existing) = db
        .get_outstanding_org_invitation_by_email(org_id, &email)
        .await
        .map_err(|e| internal("dedup invite", e))?
    {
        if existing.expires_at > chrono::Utc::now() {
            return Err(InviteError::new(
                StatusCode::CONFLICT,
                "invite_already_pending",
                "An active invitation already exists for that email",
            ));
        }
        db.revoke_org_invitation(org_id, &existing.public_id)
            .await
            .map_err(|e| internal("supersede expired invite", e))?;
    }

    // Soft capacity pre-check: reject creating invites once the org is already
    // at its member limit. The hard cap is re-enforced at acceptance
    // (`accept_invitation`), so outstanding invites are intentionally not
    // counted here — accepts beyond the limit are blocked when they land.
    let member_count = db
        .count_organization_members(org_id)
        .await
        .map_err(|e| internal("count members", e))?;
    if member_count >= max_members {
        return Err(InviteError::new(
            StatusCode::CONFLICT,
            "member_limit_reached",
            format!("Member limit reached (max {max_members})"),
        ));
    }

    let (token, token_hash) = generate_invite_token();
    let row = db
        .create_org_invitation(CreateOrgInvitation {
            public_id: generate_invite_public_id(),
            org_id,
            email: email.clone(),
            role: role.as_str().to_string(),
            invited_by: caller_user_id,
            token_hash,
            expires_at: chrono::Utc::now() + chrono::Duration::days(INVITE_TTL_DAYS),
        })
        .await
        .map_err(|e| internal("create invitation", e))?;

    let url = invite_url(frontend_url, &token);
    let email_delivery = deliver_invite_email(email_sender, org_name, &email, &url).await;

    Ok(CreatedInvitation {
        row,
        invite_url: url,
        email_delivery,
    })
}

#[derive(Debug)]
pub struct AcceptedInvitation {
    pub org_id: i64,
    pub org_public_id: String,
    pub role: String,
}

/// Validate a token and accept the invite for the authenticated principal,
/// creating local membership. Distinct error codes are returned per failure mode
/// without revealing token internals.
pub async fn accept_invitation(
    db: &StorageBackend,
    token: &str,
    user_id: Uuid,
    max_members: i64,
) -> Result<AcceptedInvitation, InviteError> {
    let invalid = || {
        InviteError::new(
            StatusCode::NOT_FOUND,
            "invite_invalid",
            "Invitation not found",
        )
    };

    if !token.starts_with(INVITE_TOKEN_PREFIX) {
        return Err(invalid());
    }

    let token_hash = hash_invite_token(token);
    let row = db
        .get_org_invitation_by_token_hash(&token_hash)
        .await
        .map_err(|e| internal("lookup invite by token", e))?
        .ok_or_else(invalid)?;

    // Resolved-state checks return distinct codes for UI handling.
    if row.revoked_at.is_some() {
        return Err(InviteError::new(
            StatusCode::CONFLICT,
            "invite_revoked",
            "This invitation has been revoked",
        ));
    }
    if row.accepted_at.is_some() {
        return Err(InviteError::new(
            StatusCode::CONFLICT,
            "invite_already_accepted",
            "This invitation has already been accepted",
        ));
    }
    if row.expires_at <= chrono::Utc::now() {
        return Err(InviteError::new(
            StatusCode::GONE,
            "invite_expired",
            "This invitation has expired",
        ));
    }

    let user = db
        .get_user(user_id)
        .await
        .map_err(|e| internal("lookup accepting user", e))?
        .ok_or_else(|| {
            InviteError::new(
                StatusCode::UNAUTHORIZED,
                "user_not_found",
                "Authenticated user not found",
            )
        })?;

    // Re-load the user row instead of trusting the authenticated session/JWT
    // email claim. Invite acceptance grants tenant membership, so the matching
    // address must be verified mailbox ownership, not a self-asserted local
    // signup address (TM-AUTH-023, TM-TENANT-011).
    if !user.email_verified {
        return Err(InviteError::new(
            StatusCode::FORBIDDEN,
            "invite_email_unverified",
            "Verify your email address before accepting this invitation",
        ));
    }

    // Authenticated, verified email must match the invited email under the same
    // normalization used at creation.
    if normalize_email(&user.email) != row.email {
        return Err(InviteError::new(
            StatusCode::FORBIDDEN,
            "invite_email_mismatch",
            "This invitation was issued to a different email address",
        ));
    }

    // Capacity guard at acceptance, matching add-member behavior.
    if !db
        .is_organization_member(row.org_id, user_id)
        .await
        .map_err(|e| internal("check membership", e))?
    {
        let member_count = db
            .count_organization_members(row.org_id)
            .await
            .map_err(|e| internal("count members", e))?;
        if member_count >= max_members {
            return Err(InviteError::new(
                StatusCode::CONFLICT,
                "member_limit_reached",
                format!("Member limit reached (max {max_members})"),
            ));
        }
    }

    // Atomically claim the invite; a concurrent accept loses here.
    let accepted = db
        .accept_org_invitation(row.id, user_id)
        .await
        .map_err(|e| internal("accept invite", e))?
        .ok_or_else(|| {
            InviteError::new(
                StatusCode::CONFLICT,
                "invite_already_accepted",
                "This invitation has already been accepted",
            )
        })?;

    // Membership is local-DB authoritative; no external identity provider call.
    db.add_organization_member(accepted.org_id, user_id, &accepted.role)
        .await
        .map_err(|e| internal("add member", e))?;

    let org = db
        .get_organization(accepted.org_id)
        .await
        .map_err(|e| internal("get organization", e))?
        .ok_or_else(|| internal("get organization", anyhow::anyhow!("org missing")))?;

    Ok(AcceptedInvitation {
        org_id: accepted.org_id,
        org_public_id: org.public_id,
        role: accepted.role,
    })
}

// ============================================================================
// HTTP handlers
// ============================================================================

/// POST /v1/orgs/{org}/invites — create an invite (Admin+).
pub async fn create_invite(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(req): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreateInviteResponse>), (StatusCode, Json<ErrorResponse>)> {
    let created = create_invitation(
        &state.db,
        state.email_sender.as_ref(),
        &state.frontend_url,
        org.org_id,
        &org.name,
        org.role,
        user.id,
        &req.email,
        req.role.as_deref(),
        state.resource_limits.max_members_per_org,
    )
    .await?;

    let mut builder = AuditEvent::management(
        ManagementAction::InvitationCreated,
        org.org_id,
        Some(user.id),
    )
    .target("invitation", &created.row.public_id)
    .detail("email", created.row.email.clone())
    .detail("role", created.row.role.clone());
    if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
        builder = builder.ip(ip);
    }
    audit::emit_event(state.db.clone(), builder.build());

    let response = CreateInviteResponse {
        invitation: to_invitation_response(&created.row),
        invite_url: created.invite_url,
        email_delivery: created.email_delivery,
    };
    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /v1/orgs/{org}/invites — list pending invites (Admin+).
pub async fn list_invites(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
) -> Result<Json<ListResponse<InvitationResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let rows = state
        .db
        .list_pending_org_invitations(org.org_id)
        .await
        .log_internal_error_json("list pending invitations")?;
    let items = rows.iter().map(to_invitation_response).collect();
    Ok(Json(ListResponse::new(items)))
}

/// DELETE /v1/orgs/{org}/invites/{invite_id} — revoke a pending invite (Admin+).
pub async fn revoke_invite(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Path((_org, invite_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let revoked = state
        .db
        .revoke_org_invitation(org.org_id, &invite_id)
        .await
        .log_internal_error_json("revoke invitation")?;
    if !revoked {
        return Err(ErrorResponse::not_found("Invitation"));
    }

    let mut builder = AuditEvent::management(
        ManagementAction::InvitationRevoked,
        org.org_id,
        Some(user.id),
    )
    .target("invitation", &invite_id);
    if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
        builder = builder.ip(ip);
    }
    audit::emit_event(state.db.clone(), builder.build());

    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/invites/{token}/accept — accept an invite (authenticated user).
pub async fn accept_invite(
    State(state): State<AppState>,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<AcceptInviteResponse>, (StatusCode, Json<ErrorResponse>)> {
    let accepted = accept_invitation(
        &state.db,
        &token,
        user.id,
        state.resource_limits.max_members_per_org,
    )
    .await?;

    // `accept_invitation` carries the real numeric org_id, so the audit event
    // always records a valid org (no public-id re-lookup that could resolve to 0).
    let org_id = accepted.org_id;

    let mut builder =
        AuditEvent::management(ManagementAction::InvitationAccepted, org_id, Some(user.id))
            .target("member", user.id.to_string())
            .detail("role", accepted.role.clone());
    if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
        builder = builder.ip(ip);
    }
    audit::emit_event(state.db.clone(), builder.build());

    Ok(Json(AcceptInviteResponse {
        org_id: accepted.org_public_id,
        role: accepted.role,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_platform::email::{EmailResult, NoopEmailSender, SentEmail};

    fn db() -> StorageBackend {
        StorageBackend::in_memory()
    }

    async fn seed_user(db: &StorageBackend, email: &str) -> Uuid {
        seed_user_with_verification(db, email, true).await
    }

    async fn seed_user_with_verification(
        db: &StorageBackend,
        email: &str,
        email_verified: bool,
    ) -> Uuid {
        let user = db
            .create_user(crate::storage::models::CreateUserRow {
                email: email.to_string(),
                name: "Test User".to_string(),
                avatar_url: None,
                roles: vec![],
                password_hash: None,
                email_verified,
                auth_provider: Some("local".to_string()),
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .expect("create user");
        user.id
    }

    async fn seed_org(db: &StorageBackend) -> i64 {
        let org = db
            .create_organization(crate::storage::models::CreateOrganizationRow {
                public_id: everruns_platform::generate_org_public_id(),
                name: "Acme".to_string(),
                created_by: None,
            })
            .await
            .expect("create org");
        org.org_id
    }

    /// EmailSender that always fails transport — exercises the delivery-failure path.
    struct FailingEmailSender;
    #[async_trait]
    impl EmailSender for FailingEmailSender {
        async fn send_email(&self, _message: EmailMessage) -> EmailResult<SentEmail> {
            Err(EmailError::Transport("boom".to_string()))
        }
    }

    /// EmailSender reporting "disabled" — exercises the not-configured path.
    struct DisabledSender;
    #[async_trait]
    impl EmailSender for DisabledSender {
        async fn send_email(&self, _message: EmailMessage) -> EmailResult<SentEmail> {
            Err(EmailError::Configuration("disabled".to_string()))
        }
    }

    #[tokio::test]
    async fn create_succeeds_without_email_service_and_returns_link() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;

        let created = create_invitation(
            &db,
            &DisabledSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            "New.Person@Example.com",
            None,
            50,
        )
        .await
        .expect("create invite");

        assert_eq!(created.email_delivery, EmailDelivery::NotConfigured);
        assert!(
            created
                .invite_url
                .starts_with("https://app.example.com/invite/evrinv_")
        );
        // Email stored normalized.
        assert_eq!(created.row.email, "new.person@example.com");
        assert_eq!(created.row.role, "member");
        // Token hash stored, never the raw token.
        assert!(!created.row.token_hash.is_empty());
        assert!(!created.row.token_hash.starts_with(INVITE_TOKEN_PREFIX));
    }

    #[tokio::test]
    async fn create_reports_sent_when_delivery_succeeds() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;

        let created = create_invitation(
            &db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Admin,
            inviter,
            "x@example.com",
            Some("admin"),
            50,
        )
        .await
        .expect("create invite");
        assert_eq!(created.email_delivery, EmailDelivery::Sent);
        assert_eq!(created.row.role, "admin");
    }

    #[tokio::test]
    async fn create_reports_failed_but_keeps_invite_pending() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;

        let created = create_invitation(
            &db,
            &FailingEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            "x@example.com",
            None,
            50,
        )
        .await
        .expect("create invite");
        assert_eq!(created.email_delivery, EmailDelivery::Failed);

        // Invite is still pending and visible.
        let pending = db.list_pending_org_invitations(org_id).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(InvitationStatus::of(&pending[0]), InvitationStatus::Pending);
    }

    #[tokio::test]
    async fn non_admin_cannot_invite_owner() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;

        let err = create_invitation(
            &db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Admin, // admin, not owner
            inviter,
            "x@example.com",
            Some("owner"),
            50,
        )
        .await
        .expect_err("admin cannot invite owner");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(err.code, "insufficient_role");
    }

    #[tokio::test]
    async fn duplicate_pending_invite_rejected() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;

        create_invitation(
            &db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            "dup@example.com",
            None,
            50,
        )
        .await
        .expect("first invite");

        let err = create_invitation(
            &db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            "dup@example.com",
            None,
            50,
        )
        .await
        .expect_err("duplicate rejected");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "invite_already_pending");
    }

    #[tokio::test]
    async fn create_supersedes_expired_invite_for_same_email() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;

        // Seed an already-expired, unrevoked invite directly (create_invitation
        // always sets a future expiry, so insert the stale row at the storage
        // layer to reproduce the post-expiry reissue path).
        db.create_org_invitation(CreateOrgInvitation {
            public_id: "orginv_000000000000000000000000000000ff".to_string(),
            org_id,
            email: "reissue@example.com".to_string(),
            role: "member".to_string(),
            invited_by: inviter,
            token_hash: "stalehash".to_string(),
            expires_at: chrono::Utc::now() - chrono::Duration::days(1),
        })
        .await
        .expect("seed expired invite");

        // Reissuing must succeed (supersede the expired row), not 500 on the
        // partial unique index.
        let created = create_invitation(
            &db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            "reissue@example.com",
            None,
            50,
        )
        .await
        .expect("reissue after expiry");

        // Exactly one outstanding (pending) invite remains.
        let pending = db.list_pending_org_invitations(org_id).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].public_id, created.row.public_id);
        assert_eq!(InvitationStatus::of(&pending[0]), InvitationStatus::Pending);
    }

    #[tokio::test]
    async fn html_escape_neutralizes_markup() {
        assert_eq!(
            html_escape("<script>&\"'"),
            "&lt;script&gt;&amp;&quot;&#39;"
        );
    }

    #[tokio::test]
    async fn accept_happy_path_creates_membership() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;
        let invitee = seed_user(&db, "invitee@example.com").await;

        // Re-derive the token: create_invitation returns the URL containing it.
        let created = create_invitation(
            &db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            "invitee@example.com",
            Some("admin"),
            50,
        )
        .await
        .expect("create invite");
        let token = created.invite_url.rsplit('/').next().unwrap().to_string();

        let accepted = accept_invitation(&db, &token, invitee, 50)
            .await
            .expect("accept invite");
        assert_eq!(accepted.role, "admin");

        assert!(db.is_organization_member(org_id, invitee).await.unwrap());

        // Now no longer pending.
        let pending = db.list_pending_org_invitations(org_id).await.unwrap();
        assert!(pending.is_empty());
    }

    async fn make_token(db: &StorageBackend, org_id: i64, inviter: Uuid, email: &str) -> String {
        let created = create_invitation(
            db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            email,
            None,
            50,
        )
        .await
        .expect("create invite");
        created.invite_url.rsplit('/').next().unwrap().to_string()
    }

    #[tokio::test]
    async fn accept_unverified_local_email_rejected() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;
        let invitee = seed_user_with_verification(&db, "invitee@example.com", false).await;
        let token = make_token(&db, org_id, inviter, "invitee@example.com").await;

        let err = accept_invitation(&db, &token, invitee, 50)
            .await
            .expect_err("unverified local account rejected");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(err.code, "invite_email_unverified");
        assert!(!db.is_organization_member(org_id, invitee).await.unwrap());
    }

    #[tokio::test]
    async fn accept_unverified_case_normalized_duplicate_email_rejected() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;
        let invitee = seed_user_with_verification(&db, "Invitee@Example.com", false).await;
        let token = make_token(&db, org_id, inviter, "invitee@example.com").await;

        let err = accept_invitation(&db, &token, invitee, 50)
            .await
            .expect_err("case-normalized unverified account rejected");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(err.code, "invite_email_unverified");
        assert!(!db.is_organization_member(org_id, invitee).await.unwrap());
    }

    #[tokio::test]
    async fn accept_wrong_email_rejected() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;
        let other = seed_user(&db, "other@example.com").await;
        let token = make_token(&db, org_id, inviter, "invitee@example.com").await;

        let err = accept_invitation(&db, &token, other, 50)
            .await
            .expect_err("wrong email rejected");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(err.code, "invite_email_mismatch");
    }

    #[tokio::test]
    async fn accept_revoked_rejected() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;
        let invitee = seed_user(&db, "invitee@example.com").await;
        let created = create_invitation(
            &db,
            &NoopEmailSender,
            "https://app.example.com",
            org_id,
            "Acme",
            OrgRole::Owner,
            inviter,
            "invitee@example.com",
            None,
            50,
        )
        .await
        .expect("create invite");
        let token = created.invite_url.rsplit('/').next().unwrap().to_string();

        assert!(
            db.revoke_org_invitation(org_id, &created.row.public_id)
                .await
                .unwrap()
        );

        let err = accept_invitation(&db, &token, invitee, 50)
            .await
            .expect_err("revoked rejected");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "invite_revoked");
    }

    #[tokio::test]
    async fn accept_twice_is_already_accepted() {
        let db = db();
        let org_id = seed_org(&db).await;
        let inviter = seed_user(&db, "admin@acme.test").await;
        let invitee = seed_user(&db, "invitee@example.com").await;
        let token = make_token(&db, org_id, inviter, "invitee@example.com").await;

        accept_invitation(&db, &token, invitee, 50)
            .await
            .expect("first accept");
        let err = accept_invitation(&db, &token, invitee, 50)
            .await
            .expect_err("second accept rejected");
        assert_eq!(err.code, "invite_already_accepted");
    }

    #[tokio::test]
    async fn accept_unknown_token_is_invalid() {
        let db = db();
        let invitee = seed_user(&db, "invitee@example.com").await;
        let err = accept_invitation(&db, "evrinv_deadbeef", invitee, 50)
            .await
            .expect_err("unknown token");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.code, "invite_invalid");
    }

    #[tokio::test]
    async fn accept_malformed_token_is_invalid() {
        let db = db();
        let invitee = seed_user(&db, "invitee@example.com").await;
        let err = accept_invitation(&db, "not-a-token", invitee, 50)
            .await
            .expect_err("malformed token");
        assert_eq!(err.code, "invite_invalid");
    }

    #[tokio::test]
    async fn token_hash_is_stable_and_not_the_raw_token() {
        let (token, hash) = generate_invite_token();
        assert!(token.starts_with(INVITE_TOKEN_PREFIX));
        assert_eq!(hash, hash_invite_token(&token));
        assert_ne!(hash, token);
    }
}
