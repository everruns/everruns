//! Serving side of an injected capability: answer one verified session's MCP
//! requests with a capability's tools, prompt, facts and hooks.
//!
//! Decisions:
//! - Transport-free: [`handle_injected_request`] takes and returns JSON-RPC
//!   values, so the control plane's HTTP route and any other host share it.
//!   The caller verifies the session token first and passes the claims in.
//! - Stateless, like the rest of our MCP endpoint: `initialize` creates no
//!   state, and every request carries its own token.
//! - A hosted capability keeps its existing [`Tool`] implementations. The
//!   service only says which tools a session gets and with which
//!   [`ToolContext`], so moving a capability to injection does not rewrite it.

use crate::MCP_PROTOCOL_VERSION_2025_06;
use crate::tool_context::ToolContext;
use crate::tools::Tool;
use anyhow::Result;
use async_trait::async_trait;
use everruns_contracts::runtime::capabilities::Fact;
use everruns_contracts::runtime::capabilities::injected::{
    InjectedCapabilityDefinition, SessionTokenClaims,
};
use everruns_contracts::runtime::mcp_extensions::{
    AfterToolParams, AfterToolResult, BeforeToolParams, BeforeToolResult, EXTENSIONS_RESOURCE_URI,
    FACTS_EXTENSION, FACTS_MIME_TYPE, FACTS_RESOURCE_URI, FactsDocument, HOOKS_AFTER_TOOL_METHOD,
    HOOKS_BEFORE_TOOL_METHOD, HOOKS_EXTENSION, HooksDeclaration, UI_EXTENSION, WireFact,
};
use serde_json::{Map, Value, json};

const PARSE_ERROR: i64 = -32700;
const INVALID_PARAMS: i64 = -32602;
const METHOD_NOT_FOUND: i64 = -32601;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;

/// A session the caller verified from its token.
#[derive(Debug, Clone)]
pub struct VerifiedSession {
    pub session_id: uuid::Uuid,
    pub claims: SessionTokenClaims,
}

impl VerifiedSession {
    /// `None` when `sub` is not a session id.
    pub fn from_claims(claims: SessionTokenClaims) -> Option<Self> {
        // Data planes put the bare UUID in `sub`; accept the public
        // `session_...` form too.
        let session_id = claims.sub.parse::<uuid::Uuid>().ok().or_else(|| {
            claims
                .sub
                .parse::<everruns_contracts::typed_id::SessionId>()
                .ok()
                .map(|id| id.uuid())
        })?;
        Some(Self { session_id, claims })
    }
}

/// One hosted capability served over MCP.
#[async_trait]
pub trait InjectedCapabilityService: Send + Sync {
    /// What the controller registers with the data plane.
    fn definition(&self) -> &InjectedCapabilityDefinition;

    /// The tools this session gets.
    async fn tools(&self, session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>>;

    /// The context the tools run with: org, session and hosted services.
    async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext>;

    /// Facts for the session's next turn. Listing any turns on the facts
    /// extension.
    async fn facts(&self, _session: &VerifiedSession) -> Result<Option<Vec<Fact>>> {
        Ok(None)
    }

    /// Tools whose calls this service wants to see.
    fn hooks(&self) -> HooksDeclaration {
        HooksDeclaration::default()
    }

    async fn before_tool(
        &self,
        _session: &VerifiedSession,
        _params: BeforeToolParams,
    ) -> Result<BeforeToolResult> {
        Ok(BeforeToolResult::Allow { arguments: None })
    }

    async fn after_tool(
        &self,
        _session: &VerifiedSession,
        _params: AfterToolParams,
    ) -> Result<AfterToolResult> {
        Ok(AfterToolResult::default())
    }

    /// Whether tool results may carry UI (`structuredContent`, `_meta`) the
    /// host renders.
    fn renders_ui(&self) -> bool {
        false
    }
}

fn success(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// The extensions object a service speaks, as served at
/// [`EXTENSIONS_RESOURCE_URI`] and in `initialize`.
async fn extensions(service: &dyn InjectedCapabilityService, session: &VerifiedSession) -> Value {
    let mut extensions = Map::new();
    if matches!(service.facts(session).await, Ok(Some(_))) {
        extensions.insert(FACTS_EXTENSION.to_string(), json!({}));
    }
    let hooks = service.hooks();
    if !hooks.is_empty() {
        extensions.insert(
            HOOKS_EXTENSION.to_string(),
            serde_json::to_value(hooks).unwrap_or_default(),
        );
    }
    if service.renders_ui() {
        extensions.insert(UI_EXTENSION.to_string(), json!({}));
    }
    Value::Object(extensions)
}

fn tool_listing(tool: &dyn Tool) -> Value {
    json!({
        "name": tool.name(),
        "description": tool.description(),
        "inputSchema": tool.parameters_schema(),
    })
}

/// Answer one JSON-RPC message. `None` for notifications, which get no reply.
pub async fn handle_injected_request(
    service: &dyn InjectedCapabilityService,
    session: &VerifiedSession,
    message: Value,
) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(failure(
            &Value::Null,
            PARSE_ERROR,
            "expected a JSON-RPC object",
        ));
    };
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(id) = object.get("id").cloned() else {
        // Notifications (`notifications/initialized`, cancellations) need no
        // answer from a stateless server.
        return None;
    };
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    Some(match dispatch(service, session, method, params).await {
        Ok(result) => success(&id, result),
        Err((code, message)) => failure(&id, code, message),
    })
}

async fn dispatch(
    service: &dyn InjectedCapabilityService,
    session: &VerifiedSession,
    method: &str,
    params: Value,
) -> std::result::Result<Value, (i64, String)> {
    let internal = |error: anyhow::Error| {
        tracing::warn!(error = %error, method, "injected capability request failed");
        (INTERNAL_ERROR, "internal error".to_string())
    };
    let definition = service.definition();
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(MCP_PROTOCOL_VERSION_2025_06);
            let mut result = json!({
                "protocolVersion": requested,
                "capabilities": {
                    "tools": {},
                    "resources": {},
                    "extensions": extensions(service, session).await,
                },
                "serverInfo": { "name": definition.name(), "version": env!("CARGO_PKG_VERSION") },
            });
            if let Some(prompt) = &definition.capability.system_prompt {
                result["instructions"] = json!(prompt);
            }
            Ok(result)
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let tools = service.tools(session).await.map_err(internal)?;
            Ok(
                json!({ "tools": tools.iter().map(|t| tool_listing(t.as_ref())).collect::<Vec<_>>() }),
            )
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or((INVALID_PARAMS, "missing tool name".to_string()))?;
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let tools = service.tools(session).await.map_err(internal)?;
            let tool = tools
                .iter()
                .find(|tool| tool.name() == name)
                .ok_or((INVALID_PARAMS, format!("unknown tool: {name}")))?;
            let context = service.tool_context(session).await.map_err(internal)?;
            let outcome = if tool.requires_context() {
                tool.execute_with_context(arguments, &context).await
            } else {
                tool.execute(arguments).await
            };
            let result = outcome.into_tool_result("injected", name);
            Ok(call_result(result))
        }
        "resources/list" => {
            let mut resources = vec![json!({
                "uri": EXTENSIONS_RESOURCE_URI, "name": "extensions",
                "mimeType": "application/json",
            })];
            if matches!(service.facts(session).await, Ok(Some(_))) {
                resources.push(json!({
                    "uri": FACTS_RESOURCE_URI, "name": "facts", "mimeType": FACTS_MIME_TYPE,
                }));
            }
            Ok(json!({ "resources": resources }))
        }
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let text = match uri {
                EXTENSIONS_RESOURCE_URI => extensions(service, session).await.to_string(),
                FACTS_RESOURCE_URI => match service.facts(session).await.map_err(internal)? {
                    Some(facts) => serde_json::to_string(&FactsDocument {
                        facts: facts.iter().map(WireFact::from).collect(),
                    })
                    .map_err(|e| internal(e.into()))?,
                    None => return Err((RESOURCE_NOT_FOUND, format!("no resource {uri}"))),
                },
                _ => return Err((RESOURCE_NOT_FOUND, format!("no resource {uri}"))),
            };
            Ok(
                json!({ "contents": [{ "uri": uri, "mimeType": "application/json", "text": text }] }),
            )
        }
        HOOKS_BEFORE_TOOL_METHOD if !service.hooks().before_tool.is_empty() => {
            let params: BeforeToolParams =
                serde_json::from_value(params).map_err(|e| (INVALID_PARAMS, e.to_string()))?;
            let result = service
                .before_tool(session, params)
                .await
                .map_err(internal)?;
            serde_json::to_value(result).map_err(|e| internal(e.into()))
        }
        HOOKS_AFTER_TOOL_METHOD if !service.hooks().after_tool.is_empty() => {
            let params: AfterToolParams =
                serde_json::from_value(params).map_err(|e| (INVALID_PARAMS, e.to_string()))?;
            let result = service
                .after_tool(session, params)
                .await
                .map_err(internal)?;
            serde_json::to_value(result).map_err(|e| internal(e.into()))
        }
        _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
    }
}

/// A tool result in MCP's `tools/call` shape: the JSON as text for the
/// model, objects also as `structuredContent`.
fn call_result(result: everruns_contracts::tool_types::ToolResult) -> Value {
    if let Some(error) = result.error {
        return json!({ "content": [{ "type": "text", "text": error }], "isError": true });
    }
    let value = result.result.unwrap_or(Value::Null);
    let text = match &value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut content = vec![json!({ "type": "text", "text": text })];
    for image in result.images.unwrap_or_default() {
        content
            .push(json!({ "type": "image", "data": image.base64, "mimeType": image.media_type }));
    }
    let mut out = json!({ "content": content, "isError": false });
    if value.is_object() {
        out["structuredContent"] = value;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolExecutionResult;
    use everruns_contracts::runtime::capabilities::injected::injected_capability_id;
    use everruns_contracts::typed_id::SessionId;

    struct Echo;

    #[async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo the text back."
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object", "properties": {"text": {"type": "string"}}})
        }
        async fn execute(&self, arguments: Value) -> ToolExecutionResult {
            match arguments.get("text").and_then(Value::as_str) {
                Some("fail") => ToolExecutionResult::tool_error("cannot echo fail"),
                Some(text) => ToolExecutionResult::success(json!({ "echo": text })),
                None => ToolExecutionResult::tool_error("missing text"),
            }
        }
    }

    struct EchoService {
        definition: InjectedCapabilityDefinition,
        facts: bool,
    }

    impl EchoService {
        fn new(facts: bool) -> Self {
            Self {
                definition: serde_json::from_value(json!({
                    "name": "echo",
                    "description": "Echo",
                    "system_prompt": "Use echo to repeat things.",
                    "server": {"url": "https://cp.example/v1/injected/echo/mcp"}
                }))
                .unwrap(),
                facts,
            }
        }
    }

    #[async_trait]
    impl InjectedCapabilityService for EchoService {
        fn definition(&self) -> &InjectedCapabilityDefinition {
            &self.definition
        }
        async fn tools(&self, _session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
            Ok(vec![Box::new(Echo)])
        }
        async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
            Ok(ToolContext::new(SessionId::from_uuid(session.session_id)))
        }
        async fn facts(&self, session: &VerifiedSession) -> Result<Option<Vec<Fact>>> {
            Ok(self
                .facts
                .then(|| vec![Fact::dynamic("echo_session", session.claims.sub.clone())]))
        }
        fn hooks(&self) -> HooksDeclaration {
            HooksDeclaration {
                before_tool: vec!["write_file".into()],
                after_tool: Vec::new(),
            }
        }
        async fn before_tool(
            &self,
            _session: &VerifiedSession,
            params: BeforeToolParams,
        ) -> Result<BeforeToolResult> {
            Ok(if params.arguments["path"] == "/etc/passwd" {
                BeforeToolResult::Deny {
                    reason: "not that file".into(),
                    user_message: None,
                }
            } else {
                BeforeToolResult::Allow { arguments: None }
            })
        }
    }

    fn session() -> VerifiedSession {
        let id = uuid::Uuid::now_v7();
        VerifiedSession::from_claims(SessionTokenClaims {
            iss: "dp".into(),
            aud: "https://cp.example/v1/injected/echo/mcp".into(),
            sub: id.to_string(),
            cap: injected_capability_id("echo"),
            grp: Some("org_1".into()),
            iat: 0,
            exp: 0,
            jti: "j".into(),
        })
        .unwrap()
    }

    async fn call(service: &EchoService, method: &str, params: Value) -> Value {
        handle_injected_request(
            service,
            &session(),
            json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn initialize_lists_instructions_and_extensions() {
        let service = EchoService::new(true);
        let result = call(
            &service,
            "initialize",
            json!({"protocolVersion": "2025-06-18"}),
        )
        .await;
        assert_eq!(result["id"], 7);
        assert_eq!(
            result["result"]["instructions"],
            "Use echo to repeat things."
        );
        let extensions = &result["result"]["capabilities"]["extensions"];
        assert!(extensions.get(FACTS_EXTENSION).is_some());
        assert_eq!(
            extensions[HOOKS_EXTENSION]["beforeTool"],
            json!(["write_file"])
        );
        assert!(extensions.get(UI_EXTENSION).is_none());
    }

    #[tokio::test]
    async fn lists_and_calls_tools() {
        let service = EchoService::new(false);
        let list = call(&service, "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"][0]["name"], "echo");
        assert_eq!(list["result"]["tools"][0]["inputSchema"]["type"], "object");

        let ok = call(
            &service,
            "tools/call",
            json!({"name": "echo", "arguments": {"text": "hi"}}),
        )
        .await;
        assert_eq!(ok["result"]["isError"], false);
        assert_eq!(ok["result"]["structuredContent"], json!({"echo": "hi"}));
        assert_eq!(ok["result"]["content"][0]["text"], r#"{"echo":"hi"}"#);

        let tool_error = call(
            &service,
            "tools/call",
            json!({"name": "echo", "arguments": {"text": "fail"}}),
        )
        .await;
        assert_eq!(tool_error["result"]["isError"], true);

        let unknown = call(
            &service,
            "tools/call",
            json!({"name": "rm", "arguments": {}}),
        )
        .await;
        assert_eq!(unknown["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn serves_facts_only_when_the_service_has_them() {
        let with = EchoService::new(true);
        let facts = call(&with, "resources/read", json!({"uri": FACTS_RESOURCE_URI})).await;
        let text = facts["result"]["contents"][0]["text"].as_str().unwrap();
        let document: FactsDocument = serde_json::from_str(text).unwrap();
        assert_eq!(document.facts[0].key, "echo_session");

        let without = EchoService::new(false);
        let missing = call(
            &without,
            "resources/read",
            json!({"uri": FACTS_RESOURCE_URI}),
        )
        .await;
        assert_eq!(missing["error"]["code"], RESOURCE_NOT_FOUND);
        let extensions = call(
            &without,
            "resources/read",
            json!({"uri": EXTENSIONS_RESOURCE_URI}),
        )
        .await;
        let text = extensions["result"]["contents"][0]["text"]
            .as_str()
            .unwrap();
        let extensions: Value = serde_json::from_str(text).unwrap();
        assert!(extensions.get(FACTS_EXTENSION).is_none());
        assert!(extensions.get(HOOKS_EXTENSION).is_some());
    }

    #[tokio::test]
    async fn runs_subscribed_hooks_only() {
        let service = EchoService::new(false);
        let deny = call(
            &service,
            HOOKS_BEFORE_TOOL_METHOD,
            json!({"toolCallId": "c", "tool": "write_file", "arguments": {"path": "/etc/passwd"}}),
        )
        .await;
        assert_eq!(deny["result"]["decision"], "deny");
        let after = call(
            &service,
            HOOKS_AFTER_TOOL_METHOD,
            json!({"toolCallId": "c", "tool": "write_file", "arguments": {}}),
        )
        .await;
        assert_eq!(after["error"]["code"], METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn notifications_get_no_reply_and_junk_gets_an_error() {
        let service = EchoService::new(false);
        assert!(
            handle_injected_request(
                &service,
                &session(),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            )
            .await
            .is_none()
        );
        let junk = handle_injected_request(&service, &session(), json!([1, 2]))
            .await
            .unwrap();
        assert_eq!(junk["error"]["code"], PARSE_ERROR);
    }

    #[test]
    fn verified_session_accepts_plain_and_prefixed_ids() {
        let id = uuid::Uuid::now_v7();
        let mut claims = session().claims;
        claims.sub = SessionId::from_uuid(id).to_string();
        assert_eq!(
            VerifiedSession::from_claims(claims.clone())
                .unwrap()
                .session_id,
            id
        );
        claims.sub = "not-a-session".into();
        assert!(VerifiedSession::from_claims(claims).is_none());
    }
}
