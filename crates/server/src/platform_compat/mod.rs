//! Legacy management URLs use the canonical router, including its authorization
//! and throttles. Rewrite before routing; never redirect writes or buffer SSE.
use axum::{
    Router,
    extract::{OriginalUri, Request},
    http::{HeaderValue, Uri, header},
    response::Response,
};
use base64::Engine;
use everruns_contracts::platform_api::{LEGACY_DEPRECATION, MIGRATION_GUIDE};
use std::{collections::HashMap, sync::Arc};
use tower::ServiceExt;

#[derive(Default)]
struct Aliases {
    platform: HashMap<String, Vec<String>>,
    published: HashMap<String, Vec<String>>,
}

fn root(path: &str) -> &str {
    path.trim_start_matches('/').split('/').next().unwrap_or("")
}

fn matches(template: &str, path: &str) -> bool {
    let mut parts = path.split('/');
    for segment in template.split('/') {
        if segment.starts_with("{*") {
            return parts.next().is_some_and(|part| !part.is_empty());
        }
        let Some(part) = parts.next() else {
            return false;
        };
        if segment.starts_with('{') && segment.ends_with('}') {
            if part.is_empty() {
                return false;
            }
        } else if segment != part {
            return false;
        }
    }
    parts.next().is_none()
}

impl Aliases {
    fn new(paths: impl IntoIterator<Item = String>) -> Self {
        let mut aliases = Self::default();
        for path in paths {
            let (table, rest) = if let Some(rest) = path.strip_prefix("/v1/platform/") {
                (&mut aliases.platform, rest)
            } else if let Some(rest) = path.strip_prefix("/v1/") {
                (&mut aliases.published, rest)
            } else {
                continue;
            };
            // OpenAPI spells catch-all file paths as ordinary parameters.
            let rest = rest.replace("/{path}", "/{*path}");
            table.entry(root(&rest).to_owned()).or_default().push(rest);
        }
        // Auth operations are absent from OpenAPI. Registered callbacks keep
        // their addresses; embedders declare any additional public receivers.
        aliases
            .platform
            .entry("auth".into())
            .or_default()
            .push("auth/{*operation}".into());
        aliases.published.entry("auth".into()).or_default().extend(
            [
                "auth/callback/{provider}",
                "auth/cli/callback",
                "auth/oidc/callback",
            ]
            .map(str::to_owned),
        );
        aliases
    }

    fn target(&self, request: &Request, prefix: &str) -> Option<Uri> {
        let rest = request
            .uri()
            .path()
            .strip_prefix(prefix)?
            .strip_prefix("/v1/")?;
        if rest == "platform" || rest.starts_with("platform/") {
            return None;
        }
        // Shared self-service URLs continue accepting runtime credentials.
        if rest == "virtual-users/me"
            || rest.starts_with("virtual-users/me/")
            || (rest.starts_with("virtual-users/") && runtime_credential(request))
        {
            return None;
        }
        let resource = root(rest);
        if self
            .published
            .get(resource)
            .is_some_and(|paths| paths.iter().any(|p| matches(p, rest)))
        {
            return None;
        }
        let platform_paths = self.platform.get(resource)?;
        // Shared agent namespaces only alias their declared management paths.
        // Other management roots include handlers absent from OpenAPI as well.
        if matches!(resource, "apps" | "channels" | "e")
            && !platform_paths.iter().any(|p| matches(p, rest))
        {
            return None;
        }
        let query = request
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        format!("{prefix}/v1/platform/{rest}{query}").parse().ok()
    }
}

fn runtime_credential(request: &Request) -> bool {
    // This is a routing hint only. RuntimeAccount validates the signature,
    // audience, binding and channel before granting any self-service access.
    request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .and_then(|(_, token)| token.split('.').nth(1))
        .and_then(|payload| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .ok()
        })
        .and_then(|payload| serde_json::from_slice::<serde_json::Value>(&payload).ok())
        .is_some_and(|claims| claims["token_type"] == "runtime_access")
}

// THREAT[TM-API-035]: canonicalize before routing; never bypass canonical auth.
/// Accept legacy paths for the supplied route inventory. Canonical handlers
/// remain the sole implementation; published protocols take precedence.
pub fn with_legacy_routes(
    router: Router,
    paths: impl IntoIterator<Item = String>,
    prefix: &str,
) -> Router {
    let aliases = Arc::new(Aliases::new(paths));
    let prefix = prefix.to_owned();
    Router::new().fallback(move |mut request: Request| {
        let router = router.clone();
        let aliases = aliases.clone();
        let prefix = prefix.clone();
        async move {
            let target = aliases.target(&request, &prefix);
            let successor = target.as_ref().map(ToString::to_string);
            if let Some(uri) = target {
                *request.uri_mut() = uri.clone();
                request.extensions_mut().insert(OriginalUri(uri));
            }
            let mut response = match router.oneshot(request).await {
                Ok(response) => response,
                Err(never) => match never {},
            };
            if let Some(successor) = successor {
                deprecation_headers(&mut response, &successor);
            }
            response
        }
    })
}

fn deprecation_headers(response: &mut Response, successor: &str) {
    response
        .headers_mut()
        .insert("deprecation", HeaderValue::from_static(LEGACY_DEPRECATION));
    if let Ok(value) = HeaderValue::from_str(&format!(
        "<{successor}>; rel=\"successor-version\", <{MIGRATION_GUIDE}>; rel=\"deprecation\"; type=\"text/html\""
    )) {
        response.headers_mut().append(header::LINK, value);
    }
}

#[cfg(test)]
mod tests;
