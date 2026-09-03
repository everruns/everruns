// Server-rendered pages that complete a URL mode elicitation started over
// `/mcp` (spec: knowledge/integrations/mcp.md, "URL mode elicitation").
//
// These pages are the whole point of URL mode: the value the MCP server needs
// is typed into a form Everruns serves and posts straight back to Everruns, so
// it never passes through the MCP client, the model's context, or the event
// log. The MCP client only ever holds the URL.
//
// Two rules govern everything here:
//
// - **The token is not a credential.** It names what to collect and for whom,
//   and grants nothing. Every page requires the visitor's own browser session
//   on top of it.
// - **The visitor must be the user the elicitation was minted for.** A URL mode
//   elicitation URL can be handed to someone else (the phishing attack in the
//   elicitation spec: one user gets another to complete their authorization, and
//   the tokens bind to the wrong identity). Verifying the signed principal
//   against the logged-in principal is what closes that, and it is enforced
//   before anything is stored or redirected.

use axum::{
    Form, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::api::mcp_endpoint::elicitation::{
    ElicitationIntent, ElicitationToken, TokenError, verify_token,
};
use crate::auth::middleware::AuthUser;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::session_storage::BatchSetSessionSecrets;
use crate::kernel_imports::Caller;
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use everruns_core::OrgRole;

use super::common::impl_auth_state;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    /// API base (`{root}/api`) used to hand a connect flow over to the existing
    /// user-connections OAuth route.
    pub api_base_url: String,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
        api_base_url: impl Into<String>,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
            api_base_url: api_base_url.into(),
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/mcp/elicitations/secret",
            get(secret_form).post(submit_secret),
        )
        .route("/mcp/elicitations/connect", get(start_connect))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    token: String,
}

#[derive(Debug, Deserialize)]
pub struct SecretForm {
    token: String,
    value: String,
}

/// GET — render the form that collects one session secret.
async fn secret_form(
    auth_user: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Response {
    match resolve(&state, &auth_user, &query.token) {
        Ok((token, ElicitationIntent::SessionSecret { session_id, name })) => page(
            StatusCode::OK,
            &secret_form_html(&query.token, &session_id, &name, &token.org_id),
        ),
        Ok(_) => page(
            StatusCode::BAD_REQUEST,
            &error_html("This link is not valid."),
        ),
        Err(error) => page(status_for(error), &error_html(error.message())),
    }
}

/// POST — store the submitted value, encrypted, and confirm.
///
/// The token is re-verified here rather than trusted from the GET: the form
/// post is a fresh request, and the same principal check must hold.
async fn submit_secret(
    auth_user: AuthUser,
    State(state): State<AppState>,
    Form(form): Form<SecretForm>,
) -> Response {
    let (token, intent) = match resolve(&state, &auth_user, &form.token) {
        Ok(resolved) => resolved,
        Err(error) => return page(status_for(error), &error_html(error.message())),
    };
    let ElicitationIntent::SessionSecret { session_id, name } = intent else {
        return page(
            StatusCode::BAD_REQUEST,
            &error_html("This link is not valid."),
        );
    };
    if form.value.is_empty() {
        return page(
            StatusCode::BAD_REQUEST,
            &error_html("Enter a value, or close this page to cancel."),
        );
    }

    let org = match resolve_org(&state, &auth_user, &token.org_id).await {
        Ok(org) => org,
        Err(message) => return page(StatusCode::FORBIDDEN, &error_html(&message)),
    };

    let stored = BatchSetSessionSecrets {
        session_id: session_id.clone(),
        secrets: HashMap::from([(name.clone(), form.value)]),
    }
    .run(&state.ctx(&org))
    .await;

    match stored {
        Ok(_) => {
            tracing::info!(
                user.id = %auth_user.id,
                org.id = %org.public_id,
                "MCP URL elicitation completed: session secret stored"
            );
            page(StatusCode::OK, &stored_html(&name, &session_id))
        }
        Err(e) => {
            // The value is in the error's blast radius, so say nothing about it.
            tracing::error!(error = %e, "Failed to store elicited session secret");
            page(
                StatusCode::BAD_REQUEST,
                &error_html("The secret could not be stored. Ask the agent to try again."),
            )
        }
    }
}

/// GET — verify the visitor is the user the elicitation was minted for, then
/// hand off to the existing user-connections OAuth flow.
///
/// This indirection is the phishing mitigation: pointing the elicitation
/// straight at the third party's authorize endpoint would let one user's link
/// bind another user's tokens.
async fn start_connect(
    auth_user: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Response {
    match resolve(&state, &auth_user, &query.token) {
        Ok((_, ElicitationIntent::Connect { provider })) => {
            tracing::info!(
                user.id = %auth_user.id,
                "MCP URL elicitation handed off to the connection flow"
            );
            Redirect::to(&format!(
                "{}/v1/user/connections/{}/authorize",
                state.api_base_url.trim_end_matches('/'),
                urlencode(&provider)
            ))
            .into_response()
        }
        Ok(_) => page(
            StatusCode::BAD_REQUEST,
            &error_html("This link is not valid."),
        ),
        Err(error) => page(status_for(error), &error_html(error.message())),
    }
}

/// Verify a presented token against the logged-in principal.
///
/// THREAT[TM-API-024]: the signed link alone grants nothing. A page only
/// renders for the principal the elicitation was minted for, so a link handed
/// to someone else cannot bind a credential to the wrong identity.
fn resolve(
    state: &AppState,
    auth_user: &AuthUser,
    token: &str,
) -> Result<(ElicitationToken, ElicitationIntent), TokenError> {
    let token = verify_token(
        token,
        &state.auth.config.jwt.secret,
        auth_user.id,
        chrono::Utc::now().timestamp(),
    )?;
    let intent = token.intent.clone();
    Ok((token, intent))
}

fn status_for(error: TokenError) -> StatusCode {
    match error {
        // Someone else's link: not a malformed request, an unauthorized one.
        TokenError::WrongPrincipal => StatusCode::FORBIDDEN,
        _ => StatusCode::BAD_REQUEST,
    }
}

/// Re-check membership against the database rather than trusting the org the
/// token names — membership can be revoked between minting and use.
async fn resolve_org(
    state: &AppState,
    auth_user: &AuthUser,
    org_public_id: &str,
) -> Result<ResolvedOrg, String> {
    let orgs = state
        .db
        .list_user_organizations(auth_user.id)
        .await
        .map_err(|_| "Could not verify your organization membership.".to_string())?;
    let org = orgs
        .iter()
        .find(|org| org.public_id == org_public_id)
        .ok_or_else(|| "You are not a member of this organization.".to_string())?;
    Ok(ResolvedOrg {
        org_id: org.org_id,
        public_id: org.public_id.clone(),
        name: org.name.clone(),
        user_id: Some(auth_user.id),
        role: org.role.parse::<OrgRole>().unwrap_or(OrgRole::Member),
        is_platform_user: auth_user.is_platform_user,
        feature_flags: everruns_platform::FeatureFlags::for_org(
            &state.auth.system_feature_flags,
            &HashMap::new(),
        ),
    })
}

// ============================================================================
// Rendering
// ============================================================================

/// Wrap a body in the document shell and the headers these pages need.
///
/// The pages are deliberately inert: no scripts, no external resources, never
/// cached, never framed, and no referrer, so the token cannot leak onward
/// through a `Referer` header.
fn page(status: StatusCode, body: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    (status, headers, document(body)).into_response()
}

fn document(body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<meta name=\"robots\" content=\"noindex, nofollow\">\
<title>Everruns</title><style>{STYLE}</style></head><body><main>{body}</main></body></html>"
    )
}

const STYLE: &str = "body{font:16px/1.5 system-ui,sans-serif;margin:0;background:#f6f7f9;color:#111}\
main{max-width:32rem;margin:4rem auto;padding:2rem;background:#fff;border-radius:12px;\
box-shadow:0 1px 3px rgba(0,0,0,.1)}h1{font-size:1.25rem;margin:0 0 .5rem}\
p{margin:.5rem 0;color:#444}label{display:block;font-weight:600;margin:1.5rem 0 .25rem}\
input[type=password]{width:100%;padding:.6rem;font:inherit;border:1px solid #ccc;border-radius:6px;\
box-sizing:border-box}button{margin-top:1.5rem;padding:.6rem 1.2rem;font:inherit;font-weight:600;\
color:#fff;background:#111;border:0;border-radius:6px;cursor:pointer}\
code{background:#f0f1f3;padding:.1rem .3rem;border-radius:4px}\
.note{font-size:.875rem;color:#666}";

fn secret_form_html(token: &str, session_id: &str, name: &str, org_id: &str) -> String {
    format!(
        "<h1>Enter a secret for Everruns</h1>\
<p>An agent asked for the value of <code>{name}</code> to use in session <code>{session}</code> \
(organization <code>{org}</code>).</p>\
<p class=\"note\">The value is encrypted and stored on the session. It is not shown to the model, \
not returned by the API, and never passes through the MCP client that asked for it.</p>\
<form method=\"post\" action=\"\" autocomplete=\"off\">\
<input type=\"hidden\" name=\"token\" value=\"{token}\">\
<label for=\"value\">{name}</label>\
<input id=\"value\" name=\"value\" type=\"password\" autocomplete=\"off\" spellcheck=\"false\" autofocus required>\
<button type=\"submit\">Save secret</button></form>\
<p class=\"note\">Close this page to cancel. Nothing is stored until you save.</p>",
        name = escape(name),
        session = escape(session_id),
        org = escape(org_id),
        token = escape(token),
    )
}

fn stored_html(name: &str, session_id: &str) -> String {
    format!(
        "<h1>Secret saved</h1>\
<p><code>{name}</code> is stored for session <code>{session}</code>.</p>\
<p class=\"note\">You can close this page and tell the agent to continue.</p>",
        name = escape(name),
        session = escape(session_id),
    )
}

fn error_html(message: &str) -> String {
    format!(
        "<h1>This request could not be completed</h1><p>{}</p>\
<p class=\"note\">You can close this page.</p>",
        escape(message)
    )
}

/// Escape text for HTML text and double-quoted attribute contexts.
fn escape(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// Percent-encode a path segment.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_html_metacharacter() {
        assert_eq!(
            escape("<script>\"x\" & 'y'</script>"),
            "&lt;script&gt;&quot;x&quot; &amp; &#39;y&#39;&lt;/script&gt;"
        );
    }

    #[test]
    fn a_hostile_secret_name_cannot_break_out_of_the_form() {
        // The name reaches the page from a tool call, so it is attacker-shaped
        // input: it must render as text, never as markup.
        let html = secret_form_html("tok", "sess_1", "\"><script>alert(1)</script>", "org_1");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn the_form_posts_the_value_to_everruns_itself() {
        let html = secret_form_html("tok.sig", "sess_1", "API_KEY", "org_1");
        // Posts back to the page's own URL. An absolute path would be wrong
        // wherever the API is mounted under a prefix (`/api` in a deployment):
        // the value would be posted at a path that does not serve this form.
        assert!(html.contains("action=\"\""));
        assert!(html.contains("method=\"post\""));
        assert!(html.contains("name=\"token\" value=\"tok.sig\""));
        // The input is a password field and never carries a prefilled value.
        assert!(html.contains("type=\"password\""));
        assert!(!html.contains("value=\"API_KEY\""));
    }

    #[test]
    fn wrong_principal_is_forbidden_not_a_bad_request() {
        assert_eq!(
            status_for(TokenError::WrongPrincipal),
            StatusCode::FORBIDDEN
        );
        assert_eq!(status_for(TokenError::Expired), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn provider_names_are_percent_encoded_into_the_redirect() {
        assert_eq!(urlencode("git hub/../x"), "git%20hub%2F..%2Fx");
    }
}
