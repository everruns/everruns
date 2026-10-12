// Personal access token scopes on `/mcp` (knowledge/security/pat-scopes.md,
// Decisions 5 and 6).
//
// Decision: a read-only PAT lists and calls only the read-only tools
//   (`tool_registry::READ_ONLY_TOOLS`); any other tool answers a tool error
//   whose envelope code is `insufficient_scope`. An unknown tool stays
//   `tool_not_found`, so the refusal reveals nothing `tools/list` does not.
// Decision: JSON-RPC methods that change state outside a tool (`tasks/cancel`,
//   `tasks/update`, `events/subscribe`, `events/unsubscribe`) are refused with
//   a JSON-RPC error carrying `data.code = "insufficient_scope"`; every other
//   method only reads or describes. Answering a parked question through
//   `session_get_status` resumes the session, so a read PAT gets the plain
//   status instead.
// Decision: a narrow token's `tools/list` is shorter than the shared catalog,
//   so it is `private` for caching; every other caller keeps `public`.
// Decision: MCP OAuth tokens, anonymous no-auth callers and `*` PATs carry no
//   narrow scope (`AuthUser.pat_scope` is `None` or `Full`) and are unaffected.

use serde_json::{Value, json};

use super::{
    AuthUser, JsonRpcRequest, JsonRpcResponse, caching, caching::CacheScope,
    classify_mcp_execute_error, error_result_payload, find_tool_definition, tool_definitions,
    tool_registry,
};
use crate::auth::pat_scope::{self, PatScope, RouteAccess};

/// JSON-RPC error code for a method a narrow personal access token may not
/// call. Implementation-defined server error range (-32000 to -32099).
pub(super) const INSUFFICIENT_SCOPE_RPC_CODE: i32 = -32003;

/// `tools/list`, with the tools the caller may call.
pub(super) fn handle_tools_list(
    id: Option<Value>,
    protocol_version: &str,
    scope: Option<PatScope>,
) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({ "tools": tools_for_scope(protocol_version, scope) }),
    )
}

/// The tools a caller with `scope` (`None`: not a PAT) may call.
fn tools_for_scope(
    protocol_version: &str,
    scope: Option<PatScope>,
) -> Vec<tool_registry::McpEndpointToolDefinition> {
    tool_definitions(protocol_version)
        .into_iter()
        .filter(|tool| pat_scope::scope_permits(scope, tool_registry::tool_access(&tool.name)))
        .collect()
}

/// Cache scope of `tools/list` for `scope`: the full catalog is the same for
/// every caller, a narrow token's is not.
pub(super) fn tools_list_cache_scope(scope: Option<PatScope>) -> CacheScope {
    if may_write(scope) {
        CacheScope::Public
    } else {
        CacheScope::Private
    }
}

/// Whether `scope` (`None`: not a PAT) may change state.
pub(super) fn may_write(scope: Option<PatScope>) -> bool {
    pat_scope::scope_permits(scope, RouteAccess::Write)
}

/// Access class of a JSON-RPC method that is not a tool call.
fn method_access(method: &str) -> RouteAccess {
    match method {
        "tasks/cancel" | "tasks/update" | "events/subscribe" | "events/unsubscribe" => {
            RouteAccess::Write
        }
        _ => RouteAccess::Read,
    }
}

/// The answer to a request outside the caller's PAT scope, or `None` to
/// handle it as usual. Checked once, before dispatch: `tools/call` of a known
/// tool by its class, every other method by `method_access`.
pub(super) fn request_denial(
    req: &JsonRpcRequest,
    protocol_version: &str,
    auth_user: &AuthUser,
) -> Option<JsonRpcResponse> {
    if req.method != "tools/call" {
        return method_denial(&req.id, &req.method, auth_user);
    }
    let name = req.params.get("name").and_then(Value::as_str)?;
    // An unknown tool stays `tool_not_found`, answered by the handler.
    find_tool_definition(name, protocol_version)?;
    let mut denied = tool_denial(&req.id, name, auth_user)?;
    if let Some(result) = denied.result.as_mut() {
        caching::mark_complete(result, protocol_version);
    }
    Some(denied)
}

/// A JSON-RPC error for a method outside the caller's PAT scope, or `None`.
fn method_denial(
    id: &Option<Value>,
    method: &str,
    auth_user: &AuthUser,
) -> Option<JsonRpcResponse> {
    pat_scope::enforce_operation(auth_user, "mcp", method, method_access(method)).err()?;
    let mut response = JsonRpcResponse::error(
        id.clone(),
        INSUFFICIENT_SCOPE_RPC_CODE,
        pat_scope::INSUFFICIENT_SCOPE_MESSAGE,
    );
    if let Some(error) = response.error.as_mut() {
        error.data = Some(json!({ "code": pat_scope::INSUFFICIENT_SCOPE }));
    }
    Some(response)
}

/// The tool error result for a tool outside the caller's PAT scope, or `None`.
fn tool_denial(
    id: &Option<Value>,
    tool_name: &str,
    auth_user: &AuthUser,
) -> Option<JsonRpcResponse> {
    pat_scope::enforce_operation(
        auth_user,
        "mcp",
        tool_name,
        tool_registry::tool_access(tool_name),
    )
    .err()?;
    let message = format!(
        "{}: {}",
        pat_scope::INSUFFICIENT_SCOPE,
        pat_scope::INSUFFICIENT_SCOPE_MESSAGE
    );
    let envelope = classify_mcp_execute_error(&message);
    Some(JsonRpcResponse::success(
        id.clone(),
        error_result_payload(&message, Some(&envelope)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;

    const LATEST: &str = super::super::MCP_PROTOCOL_VERSION_LATEST;

    fn pat(scope: PatScope) -> AuthUser {
        AuthUser {
            auth_method: AuthMethod::PersonalAccessToken,
            pat_scope: Some(scope),
            ..AuthUser::anonymous()
        }
    }

    /// An MCP OAuth token or a JWT: no PAT scope.
    fn oauth() -> AuthUser {
        AuthUser {
            auth_method: AuthMethod::Jwt,
            ..AuthUser::anonymous()
        }
    }

    fn names(scope: Option<PatScope>) -> Vec<String> {
        tools_for_scope(LATEST, scope)
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    fn all_names() -> Vec<String> {
        tool_definitions(LATEST)
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    #[test]
    fn a_read_token_lists_only_read_only_tools() {
        let mut listed = names(Some(PatScope::Read));
        listed.sort();
        let mut expected: Vec<String> = tool_registry::READ_ONLY_TOOLS
            .iter()
            .map(|name| name.to_string())
            .collect();
        expected.sort();
        assert_eq!(listed, expected);
        assert!(!listed.contains(&"execute".to_string()));
        assert_eq!(
            tools_list_cache_scope(Some(PatScope::Read)),
            CacheScope::Private
        );
    }

    #[test]
    fn full_tokens_and_other_credentials_list_every_tool() {
        assert_eq!(names(Some(PatScope::Full)), all_names());
        assert_eq!(names(None), all_names());
        assert_eq!(
            tools_list_cache_scope(Some(PatScope::Full)),
            CacheScope::Public
        );
        assert_eq!(tools_list_cache_scope(None), CacheScope::Public);
    }

    #[test]
    fn a_token_with_no_valid_scope_lists_nothing() {
        assert!(names(Some(PatScope::NoAccess)).is_empty());
    }

    #[test]
    fn a_read_token_is_refused_every_write_tool_with_insufficient_scope() {
        let id = Some(json!(1));
        let user = pat(PatScope::Read);
        let writes: Vec<String> = all_names()
            .into_iter()
            .filter(|name| !tool_registry::READ_ONLY_TOOLS.contains(&name.as_str()))
            .collect();
        for tool in ["execute", "agent_run", "session_send_message", "connect"] {
            assert!(writes.contains(&tool.to_string()), "{tool} is a write tool");
        }
        for tool in &writes {
            let denied = tool_denial(&id, tool, &user).unwrap_or_else(|| panic!("{tool}"));
            let result = denied
                .result
                .expect("a tool error result, not a JSON-RPC error");
            assert_eq!(result["isError"], json!(true), "{tool}");
            assert_eq!(
                result["structuredContent"]["code"],
                json!("insufficient_scope"),
                "{tool}: {result}"
            );
            assert_eq!(result["structuredContent"]["category"], json!("auth"));
            assert!(
                result["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains(pat_scope::INSUFFICIENT_SCOPE_MESSAGE),
                "{tool}"
            );
        }
        for tool in tool_registry::READ_ONLY_TOOLS {
            assert!(tool_denial(&id, tool, &user).is_none(), "{tool}");
        }
    }

    #[test]
    fn full_tokens_and_oauth_tokens_call_every_tool() {
        let id = Some(json!(1));
        for user in [pat(PatScope::Full), oauth(), AuthUser::anonymous()] {
            for tool in all_names() {
                assert!(tool_denial(&id, &tool, &user).is_none(), "{tool}");
            }
            for method in ["tasks/cancel", "tasks/update", "events/subscribe"] {
                assert!(method_denial(&id, method, &user).is_none(), "{method}");
            }
        }
    }

    fn request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(3)),
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn requests_are_checked_before_dispatch() {
        let read = pat(PatScope::Read);
        let call = |name: &str| request("tools/call", json!({ "name": name, "arguments": {} }));
        let denied = request_denial(&call("execute"), LATEST, &read).expect("execute refused");
        let result = denied.result.expect("tool error result");
        assert_eq!(
            result["structuredContent"]["code"],
            json!("insufficient_scope")
        );
        assert_eq!(result["resultType"], json!("complete"));
        assert!(request_denial(&call("query"), LATEST, &read).is_none());
        // Unknown tools and malformed calls are left to the handler.
        assert!(request_denial(&call("no_such_tool"), LATEST, &read).is_none());
        assert!(request_denial(&request("tools/call", json!({})), LATEST, &read).is_none());
        // Methods.
        let cancel = request("tasks/cancel", json!({ "taskId": "x" }));
        assert!(request_denial(&cancel, LATEST, &read).is_some());
        assert!(request_denial(&request("tools/list", json!({})), LATEST, &read).is_none());
        // Full tokens pass everything.
        let full = pat(PatScope::Full);
        assert!(request_denial(&call("execute"), LATEST, &full).is_none());
        assert!(request_denial(&cancel, LATEST, &full).is_none());
    }

    #[test]
    fn a_read_token_is_refused_state_changing_methods() {
        let id = Some(json!(7));
        let user = pat(PatScope::Read);
        for method in [
            "tasks/cancel",
            "tasks/update",
            "events/subscribe",
            "events/unsubscribe",
        ] {
            let denied = method_denial(&id, method, &user).unwrap_or_else(|| panic!("{method}"));
            let error = denied.error.expect("JSON-RPC error");
            assert_eq!(error.code, INSUFFICIENT_SCOPE_RPC_CODE, "{method}");
            assert_eq!(error.message, pat_scope::INSUFFICIENT_SCOPE_MESSAGE);
            assert_eq!(error.data, Some(json!({ "code": "insufficient_scope" })));
        }
        for method in ["tasks/get", "events/list", "resources/read", "tools/list"] {
            assert!(method_denial(&id, method, &user).is_none(), "{method}");
        }
        assert!(!may_write(Some(PatScope::Read)));
    }
}
