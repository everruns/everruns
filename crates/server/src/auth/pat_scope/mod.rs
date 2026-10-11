// Personal access token scopes (knowledge/security/pat-scopes.md).
//
// Decision: a token's stored `scopes` parse fail-closed into one value
//   (`PatScope`): exactly `*` is full, only `read` is read, anything else
//   authenticates but is in scope for no route. Legacy rows are never widened.
// Decision: every route gets one declared `RouteAccess`, never one inferred
//   from the HTTP method. Command-backed routes (`#[command(http = ..)]`) take
//   it from the command through the shared read-only predicate that also
//   builds the MCP `query` toolset; hand-written routes take it from the table
//   in `routes.rs`. A route with no class is unclassified, which a narrow
//   token will be denied (fail closed).
// Decision: classification happens in a router layer (`layer.rs`), which
//   knows the matched route but not the credential. The auth extractor, which
//   knows the credential, compares the two. Rollout step 1 only classifies:
//   nothing reads the `RouteAccess` extension yet, so no response changes.

mod layer;
pub mod routes;

pub use layer::classify_api_routes;

use axum::http::Method;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::domains::common::CommandDescriptor;

/// The single scope a personal access token carries, parsed from its stored
/// `scopes`. `None` on `AuthUser` for every non-PAT credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatScope {
    /// `["*"]`: full access, the behavior every PAT had before scopes.
    Full,
    /// `["read"]`: routes classified `Read` only.
    Read,
    /// Anything else (empty, unknown strings, legacy mixes such as
    /// `["read", "write"]`, `*` mixed with another scope). Authenticates, but
    /// is in scope for no route.
    NoAccess,
}

/// The scope string for full access.
pub const SCOPE_FULL: &str = "*";
/// The scope string for read-only access.
pub const SCOPE_READ: &str = "read";

impl PatScope {
    /// Parse a token's scope list, failing closed (Decision 1).
    pub fn parse<S: AsRef<str>>(scopes: &[S]) -> Self {
        let all = |wanted: &str| !scopes.is_empty() && scopes.iter().all(|s| s.as_ref() == wanted);
        if all(SCOPE_FULL) {
            Self::Full
        } else if all(SCOPE_READ) {
            Self::Read
        } else {
            Self::NoAccess
        }
    }

    /// Parse the stored `personal_access_tokens.scopes` JSON. A value that is
    /// not an array of strings is `NoAccess`, never the default.
    pub fn parse_stored(scopes: &serde_json::Value) -> Self {
        match serde_json::from_value::<Vec<String>>(scopes.clone()) {
            Ok(scopes) => Self::parse(&scopes),
            Err(_) => Self::NoAccess,
        }
    }

    /// Whether a token with this scope may reach a route of class `access`
    /// (`None`: unclassified). `PerCommand` passes the route; the command
    /// itself is checked where it is dispatched. Not enforced yet (step 2).
    pub fn permits(self, access: Option<RouteAccess>) -> bool {
        match self {
            Self::Full => true,
            Self::Read => matches!(access, Some(RouteAccess::Read | RouteAccess::PerCommand)),
            Self::NoAccess => false,
        }
    }
}

/// Declared access class of one `(method, route template)`, inserted as a
/// request extension by [`classify_api_routes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteAccess {
    /// Changes nothing: a `read` token may call it.
    Read,
    /// Changes state: full tokens only.
    Write,
    /// No scoped token may ever reach it (credentials, membership, the auth
    /// surface, the bulk data export), whatever scopes exist later.
    Never,
    /// `POST /v1/commands/{name}`: the named command decides
    /// ([`classify_command`]).
    PerCommand,
}

/// Route templates compared with path parameter names erased, so the router's
/// spelling (`{channel_id}`, `{*path}`) and the OpenAPI document's
/// (`{endpoint_id}`, `{path}`) name the same route.
pub fn normalize_template(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut in_param = false;
    for c in template.chars() {
        match c {
            '{' => {
                in_param = true;
                out.push_str("{}");
            }
            '}' => in_param = false,
            _ if in_param => {}
            _ => out.push(c),
        }
    }
    out
}

/// Normalize a method for classification: `HEAD` is served by the `GET`
/// handler, so it shares its class.
fn method_key(method: &Method) -> Method {
    if method == Method::HEAD {
        Method::GET
    } else {
        method.clone()
    }
}

type RouteKey = (Method, String);

fn route_key(method: &str, template: &str) -> RouteKey {
    let method = Method::from_bytes(method.as_bytes()).expect("valid HTTP method");
    (method_key(&method), normalize_template(template))
}

/// Every hand-written classification, documented or not, by normalized key.
static TABLE: LazyLock<HashMap<RouteKey, RouteAccess>> = LazyLock::new(|| {
    routes::ROUTES
        .iter()
        .chain(routes::UNDOCUMENTED_ROUTES)
        .map(|(method, path, access)| (route_key(method, path), *access))
        .collect()
});

/// Every command served by the generic HTTP handler, by its route.
static HTTP_COMMANDS: LazyLock<HashMap<RouteKey, &'static CommandDescriptor>> =
    LazyLock::new(|| {
        let names: std::collections::HashSet<&str> =
            crate::api::command_http::http_command_names().collect();
        inventory::iter::<CommandDescriptor>
            .into_iter()
            .filter_map(|desc| {
                let meta = (desc.meta)();
                names
                    .contains(meta.name)
                    .then(|| (route_key(meta.method, meta.path), desc))
            })
            .collect()
    });

/// The command served at a route by the generic HTTP handler, if any.
pub fn http_command_at(method: &Method, template: &str) -> Option<&'static CommandDescriptor> {
    HTTP_COMMANDS
        .get(&(method_key(method), normalize_template(template)))
        .copied()
}

/// Class of one command, wherever it is invoked (its own route or
/// `POST /v1/commands/{name}`): `Never` when its route is on the Never list,
/// else `Read` exactly when the shared read-only predicate holds.
pub fn classify_command(desc: &CommandDescriptor) -> RouteAccess {
    let meta = (desc.meta)();
    if TABLE.get(&route_key(meta.method, meta.path)) == Some(&RouteAccess::Never) {
        return RouteAccess::Never;
    }
    if crate::services::command_catalog::catalog::is_read_only_command(desc) {
        RouteAccess::Read
    } else {
        RouteAccess::Write
    }
}

/// Class of `(method, template)`, where `template` is the matched route with
/// the API prefix removed (`/v1/...`). `None` means unclassified.
pub fn classify(method: &Method, template: &str) -> Option<RouteAccess> {
    let key = (method_key(method), normalize_template(template));
    let entry = TABLE.get(&key).copied();
    if entry == Some(RouteAccess::Never) {
        return entry;
    }
    if let Some(desc) = HTTP_COMMANDS.get(&key) {
        return Some(classify_command(desc));
    }
    entry.or_else(|| {
        routes::NEVER_PREFIXES
            .iter()
            .any(|prefix| key.1.starts_with(prefix))
            .then_some(RouteAccess::Never)
    })
}

#[cfg(test)]
mod tests;
