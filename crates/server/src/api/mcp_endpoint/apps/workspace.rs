//! Shared product UI resources and entity entrypoints.
use super::super::{
    AppState, AuthUser, JsonRpcResponse, ResolvedOrg, dispatch_command, error_result_payload,
    resolve_org_override,
};
use serde_json::{Value, json};

pub(super) const HARNESS_URI: &str = "ui://everruns/app/harness";
pub(super) const CAPABILITY_URI: &str = "ui://everruns/app/capability";
pub(super) const KNOWLEDGE_URI: &str = "ui://everruns/app/knowledge";
const SHELL: &str = include_str!("workspace.html");

pub(super) fn entity_tool(name: &str) -> bool {
    matches!(
        name,
        "harness_get_card" | "capability_get_card" | "knowledge_get_card" | "session_open"
    )
}

pub(crate) async fn handle(
    id: Option<Value>,
    name: &str,
    args: &Value,
    user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
) -> Option<JsonRpcResponse> {
    if !entity_tool(name) && !super::bridge::is_tool(name) {
        return None;
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        // THREAT[TM-MCP-004]: the parent pins its current organization on every
        // bridge call. This identity echo is not a PAT organization override;
        // a different organization still goes through the normal guard.
        let org = if super::bridge::is_tool(name)
            && args.get("organization_id").and_then(Value::as_str) == Some(org.public_id.as_str())
        {
            org.clone()
        } else {
            resolve_org_override(args, user, org, state).await?
        };
        if super::bridge::is_tool(name) {
            return super::bridge::call(name, args, user, &org, state).await;
        }
        let (kind, operation, field, arg) = match name {
            "harness_get_card" => ("harness", "get_harness", "id", "harness_id"),
            "capability_get_card" => ("capability", "get_capability", "id", "capability_id"),
            "knowledge_get_card" => (
                "knowledge",
                "get_knowledge",
                "collection_id",
                "knowledge_id",
            ),
            _ => ("session", "get_session", "session_id", "session_id"),
        };
        let value = args
            .get(arg)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Missing {arg}"))?;
        if kind == "knowledge" && !org.feature_flags.knowledge {
            return Err("Knowledge is not enabled for this organization".into());
        }
        let entity = dispatch_command(operation, json!({field: value}), &org, state).await?;
        if serde_json::to_vec(&entity)
            .map_err(|e| e.to_string())?
            .len()
            > super::bridge::MAX_BODY
        {
            return Err("Entity exceeds the workspace size limit".into());
        }
        let id = entity.get("id").and_then(Value::as_str).unwrap_or(value);
        Ok(json!({
            "view": {"kind": kind, "id": id},
            "organization_id": org.public_id,
            "entity": entity
        }))
    })
    .await
    .unwrap_or_else(|_| Err("Workspace request timed out".into()));
    Some(match result {
        Ok(view) => JsonRpcResponse::success(id, super::super::json_result_payload(&view)),
        Err(error) => JsonRpcResponse::success(id, error_result_payload(&error, None)),
    })
}

pub(crate) fn resource(uri: &str, frontend: &str) -> Option<Value> {
    let kind = match uri {
        super::HOME_VIEW_URI => "home",
        super::SESSION_VIEW_URI => "session",
        super::AGENT_VIEW_URI => "agent",
        HARNESS_URI => "harness",
        CAPABILITY_URI => "capability",
        KNOWLEDGE_URI => "knowledge",
        _ => return None,
    };
    let url = url::Url::parse(frontend).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    let base = format!("{origin}/mcp-app");
    // Configuration is data, never executable HTML, including a configured URL
    // containing a script closing sequence.
    let config = json!({"base":base,"origin":origin,"kind":kind})
        .to_string()
        .replace('<', "\\u003c");
    let html = SHELL
        .replace("__WORKSPACE_CONFIG__", &config)
        .replace("__WORKSPACE_ORIGIN__", &origin);
    Some(
        json!({ "contents": [{"uri":uri,"mimeType":super::APP_MIME_TYPE,"text":html,"_meta":{
        "ui":{"csp":{"connectDomains":[],"resourceDomains":[],"frameDomains":[origin]},"permissions":{"clipboardWrite":{}},"prefersBorder":true},
        "openai/ui":{"availableDisplayModes":["inline","fullscreen"],"preferredDisplayMode":if matches!(kind,"home"|"session"){"fullscreen"}else{"inline"}}
    }}] }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_origin_and_resource_metadata_match_for_every_surface() {
        for uri in [
            super::super::AGENT_VIEW_URI,
            super::super::HOME_VIEW_URI,
            super::super::SESSION_VIEW_URI,
            HARNESS_URI,
            CAPABILITY_URI,
            KNOWLEDGE_URI,
        ] {
            let resource = resource(uri, "https://app.example.test/?value=</script>").unwrap();
            let content = &resource["contents"][0];
            let html = content["text"].as_str().unwrap();
            assert_eq!(
                content["_meta"]["ui"]["csp"]["frameDomains"],
                json!(["https://app.example.test"])
            );
            assert!(html.contains("https://app.example.test/mcp-app"));
            assert!(!html.contains("__WORKSPACE_"));
            assert!(!html.contains("?value="));
            assert!(!html.contains("innerHTML"));
        }
        assert!(resource(HARNESS_URI, "javascript:alert(1)").is_none());
    }
}
