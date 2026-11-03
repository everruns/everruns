//! Authenticated, bounded UI API reads/writes over the MCP Apps host bridge.
//! The widget cannot choose an origin, headers, credentials, or organization.
use super::super::{AppState, AuthUser, ResolvedOrg};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request},
};
use base64::Engine as _;
use serde_json::{Value, json};
use tower::ServiceExt;

pub(super) const READ: &str = "app_read";
pub(super) const WRITE: &str = "app_write";
pub(super) const BOOTSTRAP: &str = "app_bootstrap";
pub(super) const MAX_BODY: usize = 8 * 1024 * 1024;

pub(super) fn is_tool(name: &str) -> bool {
    matches!(name, READ | WRITE | BOOTSTRAP)
}

/// Only product data used by Home, entity cards and session workspaces. Auth,
/// credentials, org administration, command execution and external URLs stay out.
fn allowed_path(raw: &str, read: bool) -> bool {
    let path = raw.split('?').next().unwrap_or_default();
    let lower = path.to_ascii_lowercase();
    let workspace_files =
        path.split('/').nth(2) == Some("workspaces") && path.split('/').nth(4) == Some("fs");
    if !path.starts_with("/v1/")
        || path.contains('\\')
        || path.contains("//")
        || lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains("%2e")
        || (!workspace_files && lower.contains("%25"))
        || path.split('/').any(|p| p == "." || p == "..")
    {
        return false;
    }
    // Workspace reads retain the filesystem API's containment and tenant checks.
    // Filenames are data here, including names such as "keys" or "secrets".
    if workspace_files {
        return read;
    }
    // The recording UI lists names/metadata only; secret values and credential
    // writes must use the existing credential-collection surface.
    let segments: Vec<_> = path.split('/').collect();
    if segments.len() == 6
        && segments[2] == "sessions"
        && segments[4] == "storage"
        && matches!(segments[5], "keys" | "secrets")
    {
        return read;
    }
    if matches!(path, "/v1/durable/config" | "/v1/system/doctor") {
        return read;
    }
    let resource = path.split('/').nth(2).unwrap_or_default();
    matches!(
        resource,
        "agents"
            | "harnesses"
            | "capabilities"
            | "knowledge"
            | "sessions"
            | "models"
            | "providers"
            | "agent-examples"
            | "health-issues"
            | "policies"
            | "sandbox-fleet"
            | "sandboxes"
            | "monitoring"
            | "feature-flags"
            | "virtual-users"
            | "session-schedules"
            | "user-preferences"
            | "background-tasks"
    ) && !path.split('/').any(|part| {
        matches!(
            part,
            "credentials" | "secrets" | "keys" | "token" | "tokens" | "sse"
        )
    })
}

pub(super) async fn call(
    name: &str,
    args: &Value,
    user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    if name == BOOTSTRAP {
        return Ok(json!({
            "user": { "id": user.id, "name": user.name, "email": user.email, "roles": user.roles },
            "organization": { "public_id": org.public_id, "name": org.name, "role": org.role },
            "feature_flags": org.feature_flags,
            "read_only": org.read_only,
        }));
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("Missing path")?;
    if !allowed_path(path, name == READ) {
        return Err("This API is not available in the MCP workspace".into());
    }
    let method = if name == READ {
        Method::GET
    } else {
        let raw = args
            .get("method")
            .and_then(Value::as_str)
            .ok_or("Missing method")?;
        match raw {
            "POST" => Method::POST,
            "PATCH" => Method::PATCH,
            "PUT" => Method::PUT,
            "DELETE" => Method::DELETE,
            _ => return Err("Invalid write method".into()),
        }
    };
    if name == WRITE && org.read_only {
        return Err("insufficient_scope: read-only token".into());
    }
    let body = args
        .get("body")
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if body.len() > MAX_BODY {
        return Err("Request exceeds the workspace size limit".into());
    }
    let router = state
        .app_router
        .clone()
        .ok_or("Embedded UI API routes are not configured")?;
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-org-id", &org.public_id)
        .extension(crate::auth::mcp_app::AppCaller(user.clone(), org.clone()));
    for header in ["everruns-change-reason", "everruns-context-revision"] {
        if let Some(value) = args
            .get("headers")
            .and_then(|headers| headers.get(header))
            .and_then(Value::as_str)
        {
            request = request.header(header, value);
        }
    }
    let request = request.body(Body::from(body)).map_err(|e| e.to_string())?;
    let response = router.oneshot(request).await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let headers: serde_json::Map<String, Value> = [
        "content-type",
        "x-total-count",
        "content-disposition",
        "everruns-committed-change",
        "x-atif-images-omitted",
        "x-atif-next-cursor",
    ]
    .into_iter()
    .filter_map(|name| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| (name.into(), json!(v)))
    })
    .collect();
    let bytes = to_bytes(response.into_body(), MAX_BODY)
        .await
        .map_err(|_| "Response exceeds the workspace size limit")?;
    Ok(
        json!({ "status": status, "headers": headers, "body": base64::engine::general_purpose::STANDARD.encode(bytes) }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_paths_are_local_and_exclude_credentials_and_recursion() {
        for path in [
            "/v1/sessions/session_1/trace?limit=20",
            "/v1/agents/activity",
            "/v1/knowledge",
            "/v1/workspaces/ws_1/fs/workspace/report%25.txt",
            "/v1/workspaces/ws_1/fs/_/download/workspace/keys",
            "/v1/sessions/session_1/storage/secrets",
            "/v1/sessions/session_1/storage/keys",
            "/v1/durable/config",
            "/v1/system/doctor",
        ] {
            assert!(allowed_path(path, true), "{path}");
        }
        for path in [
            "https://example.com/v1/agents",
            "//example.com/v1/agents",
            "/mcp",
            "/v1/auth/me",
            "/v1/commands/run",
            "/v1/agents/../auth/me",
            "/v1/agents/%2e%2e/auth/me",
            "/v1/agents/agent_1/credentials",
            "/v1/sessions/session_1/sse",
        ] {
            assert!(!allowed_path(path, true), "{path}");
        }
    }
}
