// Enforcement of personal access token scopes
// (knowledge/security/pat-scopes.md, Decisions 3 and 6).
//
// Decision: the auth extractor is the only place a route scope decision is
//   made. Right after a PAT validates, `enforce_route` compares its parsed
//   scope with the `RouteAccess` extension the classification layer inserted.
//   A missing extension is an unclassified route, denied to a narrow token. A
//   `*` token, a JWT and a cookie (no `pat_scope`) skip the check entirely.
// Decision: `POST /v1/commands/{name}` (`PerCommand`) passes the route check;
//   `execute_command` calls `enforce_command` with the named command. The
//   extractor leaves the token's scope in the request extensions for that.
// Decision: a denial is a 403 `insufficient_scope` with a fixed message and an
//   RFC 6750 `WWW-Authenticate` challenge, one `warn!` naming the user, the
//   token's display prefix (never the token), the route and its class, and one
//   `pat_scope_denied_total{class}` increment. Not written to the org audit log:
//   a PAT often has no resolved org here, and a looping client would flood it.

use axum::extract::MatchedPath;
use axum::http::{Extensions, HeaderValue, Method, StatusCode, header};
use axum::response::Response;

use super::{PatScope, RouteAccess, classify_command};
use crate::auth::middleware::{AuthError, AuthUser};
use crate::auth::personal_access_token::PAT_PREFIX;
use crate::domains::common::CommandDescriptor;

/// RFC 6750 error code for a token that lacks the scope a request needs.
pub const INSUFFICIENT_SCOPE: &str = "insufficient_scope";

/// The fixed message of every scope denial: no route or permission detail.
pub const INSUFFICIENT_SCOPE_MESSAGE: &str =
    "This personal access token is read-only; this operation needs a full-access token.";

const CHALLENGE: &str = "Bearer error=\"insufficient_scope\"";

impl AuthError {
    /// 403 for a personal access token whose scope does not cover the request.
    pub fn insufficient_scope() -> Self {
        Self {
            error: INSUFFICIENT_SCOPE_MESSAGE.to_string(),
            status: StatusCode::FORBIDDEN,
            code: Some(INSUFFICIENT_SCOPE),
        }
    }
}

/// Add the RFC 6750 §3.1 challenge to an `insufficient_scope` response.
pub(crate) fn add_challenge(code: Option<&str>, response: &mut Response) {
    if code == Some(INSUFFICIENT_SCOPE) {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(CHALLENGE),
        );
    }
}

/// Check a freshly validated PAT user against the route's class. `token` is
/// the presented token, used only for its non-secret display prefix in the
/// denial log.
pub(crate) fn enforce_route(
    extensions: &mut Extensions,
    method: &Method,
    token: &str,
    user: AuthUser,
) -> Result<AuthUser, AuthError> {
    let Some(scope) = user.pat_scope else {
        return Ok(user);
    };
    // Left for `execute_command`, which checks the named command.
    extensions.insert(scope);
    if scope == PatScope::Full {
        return Ok(user);
    }
    let access = extensions.get::<RouteAccess>().copied();
    if scope.permits(access) {
        return Ok(user);
    }
    let route = extensions
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string())
        .unwrap_or_default();
    deny(
        &user,
        token_display_prefix(token),
        method.as_str(),
        &route,
        access,
    )
}

/// Check the command named by `POST /v1/commands/{name}` against the scope
/// the extractor left in the request extensions (`None`: not a PAT, or a
/// full one). Unknown commands are not checked here; they stay 404.
pub fn enforce_command(
    scope: Option<PatScope>,
    user_id: Option<uuid::Uuid>,
    desc: &CommandDescriptor,
) -> Result<(), AuthError> {
    let Some(scope) = scope.filter(|scope| *scope != PatScope::Full) else {
        return Ok(());
    };
    let class = classify_command(desc);
    if scope == PatScope::Read && class == RouteAccess::Read {
        return Ok(());
    }
    let name = (desc.meta)().name;
    tracing::warn!(
        user_id = ?user_id,
        scope = ?scope,
        command = name,
        class = class_label(Some(class)),
        "personal access token scope denied a command"
    );
    count_denial(Some(class));
    Err(AuthError::insufficient_scope())
}

fn deny(
    user: &AuthUser,
    token_prefix: &str,
    method: &str,
    route: &str,
    access: Option<RouteAccess>,
) -> Result<AuthUser, AuthError> {
    tracing::warn!(
        user_id = %user.id,
        token_prefix,
        scope = ?user.pat_scope,
        method,
        route,
        class = class_label(access),
        "personal access token scope denied a request"
    );
    count_denial(access);
    Err(AuthError::insufficient_scope())
}

fn count_denial(access: Option<RouteAccess>) {
    metrics::counter!(
        crate::metrics_names::PAT_SCOPE_DENIED_TOTAL,
        "class" => class_label(access),
    )
    .increment(1);
}

fn class_label(access: Option<RouteAccess>) -> &'static str {
    match access {
        Some(RouteAccess::Read) => "read",
        Some(RouteAccess::Write) => "write",
        Some(RouteAccess::Never) => "never",
        Some(RouteAccess::PerCommand) => "per_command",
        None => "unclassified",
    }
}

/// The token's display prefix, as stored in `personal_access_tokens.token_prefix`
/// and shown in the token list: `evr_pat_` plus 8 characters. Identifies the
/// token to its owner without logging the secret.
fn token_display_prefix(token: &str) -> &str {
    let end = PAT_PREFIX.len() + 8;
    token.get(..end).unwrap_or(PAT_PREFIX)
}
