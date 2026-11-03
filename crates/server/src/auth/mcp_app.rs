//! In-process identity handoff from an authenticated MCP call to the shared UI APIs.
//! These extensions are never constructed from HTTP headers or widget arguments.
use super::{
    ResolvedOrg,
    middleware::{AuthError, AuthUser},
};
use axum::http::request::Parts;

#[derive(Clone)]
pub(crate) struct AppCaller(pub AuthUser, pub ResolvedOrg);

pub(crate) fn user(parts: &mut Parts) -> Option<Result<AuthUser, AuthError>> {
    let caller = parts.extensions.get::<AppCaller>()?.clone();
    // Preserve the REST route's declared PAT scope even though authentication
    // already happened at /mcp. No MCP token is accepted by the HTTP audience.
    Some(super::pat_scope::enforce_route(
        &mut parts.extensions,
        &parts.method,
        "",
        caller.0,
    ))
}

pub(crate) fn org(parts: &Parts) -> Option<ResolvedOrg> {
    parts
        .extensions
        .get::<AppCaller>()
        .map(|caller| caller.1.clone())
}
