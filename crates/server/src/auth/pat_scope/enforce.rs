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
// Decision: surfaces that validate a PAT outside the extractor (`/mcp` tools
//   and methods, the execution API's `org_member`) check one named operation
//   with `enforce_operation`, which logs and counts the same way. Listings
//   (`tools/list`, `GET /v1/commands`) filter with `scope_permits` silently:
//   leaving out what a token cannot run is not a denial.
// Decision: `Command::run` repeats the check for a `Caller` marked read-only
//   (`enforce_caller`), behind every surface above. It covers a handler that
//   dispatches a write command behind a route misclassified as `Read`, so a
//   denial here means a classification gap: it logs at `warn!` with
//   `class="caller"` and the same metric.

use axum::extract::MatchedPath;
use axum::http::{Extensions, HeaderValue, Method, StatusCode, header};
use axum::response::Response;

use super::{PatScope, RouteAccess, classify_command};
use crate::auth::middleware::{AuthError, AuthUser};
use crate::auth::personal_access_token::PAT_PREFIX;
use crate::domains::common::{CommandDescriptor, CommandError};

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

/// Whether a credential with `scope` (`None`: not a PAT) may run an operation
/// of class `access` that is not a routed request: an MCP tool or method, a
/// command in a listing, a member call on the execution API.
pub fn scope_permits(scope: Option<PatScope>, access: RouteAccess) -> bool {
    scope.is_none_or(|scope| scope.permits(Some(access)))
}

/// Check one named operation of class `access` on a surface that validates
/// PATs outside the extractor (`surface`: `mcp`, `agent_api`). JWTs, MCP OAuth
/// tokens and `*` PATs always pass.
pub fn enforce_operation(
    user: &AuthUser,
    surface: &'static str,
    operation: &str,
    access: RouteAccess,
) -> Result<(), AuthError> {
    if scope_permits(user.pat_scope, access) {
        return Ok(());
    }
    tracing::warn!(
        user_id = %user.id,
        scope = ?user.pat_scope,
        surface,
        operation,
        class = class_label(Some(access)),
        "personal access token scope denied an operation"
    );
    count_denial(Some(access));
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

/// Defense in depth inside `Command::run`: refuse a command that is not
/// read-only for a caller whose credential is read-only (`Caller.read_only`).
/// The route and per-command checks should have refused it first.
pub(crate) fn enforce_caller(
    caller: &everruns_core::Caller,
    desc: &CommandDescriptor,
) -> Result<(), CommandError> {
    if !caller.read_only || classify_command(desc) == RouteAccess::Read {
        return Ok(());
    }
    tracing::warn!(
        user_id = ?caller.user_id,
        org_id = caller.org_id,
        command = (desc.meta)().name,
        "read-only caller reached a write command past the route scope check"
    );
    metrics::counter!(crate::metrics_names::PAT_SCOPE_DENIED_TOTAL, "class" => "caller")
        .increment(1);
    Err(CommandError::forbidden(INSUFFICIENT_SCOPE_MESSAGE).with_code(INSUFFICIENT_SCOPE))
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
