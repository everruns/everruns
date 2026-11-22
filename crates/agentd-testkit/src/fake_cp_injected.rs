//! The fake control plane's injected servers (`injected::CONTROLLER_SERVERS`):
//! what a session's native tools ask the control plane at call time, as
//! MCP over HTTP with a session token the data plane signed. Each call is
//! recorded by the v1 worker command the real control plane runs for it,
//! with the session, the person and the turn taken from the token, and
//! answered from the answers a test set ([`super::FakeCp::set_lookup`]).

use std::collections::BTreeMap;

use anyhow::Result;
use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use everruns_contracts::runtime::capabilities::injected::{
    CONTROLLER_SERVERS, CREDENTIAL_TOOL, CREDENTIALS, InjectedCapabilityDefinition, MEMORY_FILES,
    PERSON_MCP, PERSON_MCP_SERVERS_TOOL, PERSON_MCP_STORE_TOOL, PLATFORM_COMMAND_TOOL,
    PLATFORM_COMMANDS, injected_capability_id,
};
use everruns_contracts::tools::ToolExecutionResult;
use everruns_contracts::typed_id::SessionId;
use everruns_core::mcp::injected_server::{
    InjectedCapabilityService, VerifiedSession, handle_injected_request,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::Tool;
use serde_json::{Value, json};

use super::{Answer, FakeCp};

/// The v1 worker command a credential call runs.
fn credential_command(call: &str) -> &'static str {
    match call {
        "connection_token" => "worker_get_connection_token",
        "mcp_connection_token" => "worker_get_mcp_connection_token",
        "github_token" => "worker_get_github_token",
        "service_api_key" => "worker_get_service_api_key_connection",
        _ => "unknown_credential",
    }
}

/// The v1 worker command a platform commands operation runs.
fn platform_command(operation: &str) -> &'static str {
    match operation {
        "spend" => "worker_get_session_spend",
        "scripts" => "list_agent_scripts",
        "save_script" => "agent_script_save",
        _ => "platform_command_surface",
    }
}

impl FakeCp {
    /// The URLs of its injected servers, by name, as a create carries them
    /// (`servers`).
    pub fn servers(&self) -> Value {
        let servers: BTreeMap<&str, String> = CONTROLLER_SERVERS
            .iter()
            .map(|name| (*name, self.server_url(name)))
            .collect();
        json!(servers)
    }

    fn server_url(&self, name: &str) -> String {
        format!("{}/injected/{name}/mcp", self.url)
    }

    /// Verify the session tokens of a data plane whose key is `key`.
    /// Panics on a key it cannot use: test setup.
    #[allow(clippy::expect_used)]
    pub fn trust_data_plane(&self, key: &everruns_contracts::dataplane_seal::Key) {
        let signer = everruns_core::mcp::session_token::SessionTokenSigner::from_pkcs8(
            everruns_contracts::dataplane_seal::SESSION_TOKEN_ISSUER,
            &everruns_contracts::dataplane_seal::session_token_pkcs8(key),
        )
        .expect("a session token key");
        let verifier = everruns_core::mcp::session_token::SessionTokenVerifier::from_jwks(
            &signer.jwks(),
            vec![signer.issuer().into()],
        )
        .expect("a verifier");
        *self.verifier.lock() = Some(std::sync::Arc::new(verifier));
    }

    /// Record `command` with `params` for the token's org and answer it.
    fn ask(&self, session: &VerifiedSession, command: &str, mut params: Value) -> Answer {
        params["session_id"] = Value::String(SessionId::from_uuid(session.session_id).to_string());
        params["acting_user"] = Value::Null;
        params["input_message_id"] = Value::Null;
        if let Some(act) = &session.claims.act {
            params["acting_user"] = Value::String(act.sub.clone());
            if let Some(turn) = &act.turn {
                params["input_message_id"] = Value::String(turn.clone());
            }
        }
        let org = session.claims.grp.clone().unwrap_or_default();
        let mut ledger = self.ledger.lock();
        ledger.lookups.push((org, command.to_owned(), params));
        ledger
            .lookup_answers
            .get(command)
            .cloned()
            .unwrap_or(Answer::Error {
                kind: 3,
                message: "Session not found".into(),
            })
    }
}

/// `POST /injected/{name}/mcp`: one JSON-RPC message, with a session token
/// for the server.
pub(super) async fn serve(
    State(cp): State<FakeCp>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if cp.knobs.down.load(std::sync::atomic::Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Some(verifier) = cp.verifier.lock().clone() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let bearer = everruns_agentd::org_token::bearer(&headers).unwrap_or_default();
    let Ok(claims) = verifier.verify(
        bearer,
        &cp.server_url(&name),
        &injected_capability_id(&name),
    ) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(session) = VerifiedSession::from_claims(claims) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(definition) = serde_json::from_value(json!({
        "name": name,
        "description": name,
        "server": { "url": cp.server_url(&name) },
    })) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let service = Server {
        definition,
        name,
        cp,
    };
    match handle_injected_request(&service, &session, message).await {
        Some(reply) => axum::Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

struct Server {
    name: String,
    definition: InjectedCapabilityDefinition,
    cp: FakeCp,
}

#[async_trait]
impl InjectedCapabilityService for Server {
    fn definition(&self) -> &InjectedCapabilityDefinition {
        &self.definition
    }

    async fn tools(&self, session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
        let names: &[&str] = match self.name.as_str() {
            CREDENTIALS => &[CREDENTIAL_TOOL],
            PLATFORM_COMMANDS => &[PLATFORM_COMMAND_TOOL],
            PERSON_MCP => &[PERSON_MCP_STORE_TOOL, PERSON_MCP_SERVERS_TOOL],
            _ => &[],
        };
        Ok(names
            .iter()
            .map(|tool| -> Box<dyn Tool> {
                Box::new(Call {
                    tool: tool.to_string(),
                    cp: self.cp.clone(),
                    session: session.clone(),
                })
            })
            .collect())
    }

    async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
        Ok(ToolContext::new(SessionId::from_uuid(session.session_id)))
    }

    fn serves_files(&self) -> bool {
        self.name == MEMORY_FILES
    }

    async fn files(
        &self,
        session: &VerifiedSession,
        op: &'static str,
        mut params: Value,
    ) -> Result<Value> {
        params["op"] = Value::String(op.to_string());
        match self.cp.ask(session, "memory_file", params) {
            Answer::Value(answer) => match answer.get("error").and_then(Value::as_str) {
                Some(error) => Err(anyhow::anyhow!("{error}")),
                None => Ok(answer.get("ok").cloned().unwrap_or(Value::Null)),
            },
            Answer::Error { message, .. } => Err(anyhow::anyhow!("{message}")),
        }
    }
}

/// One of the servers' tools.
struct Call {
    tool: String,
    cp: FakeCp,
    session: VerifiedSession,
}

fn text(value: Value) -> ToolExecutionResult {
    ToolExecutionResult::success(Value::String(value.to_string()))
}

#[async_trait]
impl Tool for Call {
    fn name(&self) -> &str {
        &self.tool
    }

    fn description(&self) -> &str {
        "fake control plane call"
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        match self.tool.as_str() {
            CREDENTIAL_TOOL => {
                let call = arguments["call"].as_str().unwrap_or_default();
                let params = match arguments.get("params") {
                    Some(Value::Object(params)) => Value::Object(params.clone()),
                    _ => json!({}),
                };
                match self.cp.ask(&self.session, credential_command(call), params) {
                    Answer::Value(value) => text(json!({ "ok": value })),
                    Answer::Error { kind, message } => {
                        text(json!({ "error": { "kind": kind, "message": message } }))
                    }
                }
            }
            PLATFORM_COMMAND_TOOL => {
                let operation = arguments["operation"].as_str().unwrap_or_default();
                let command = platform_command(operation);
                let inner = arguments.get("arguments").cloned().unwrap_or(json!({}));
                let params = if command == "platform_command_surface" {
                    json!({ "operation": operation, "arguments": inner })
                } else {
                    inner
                };
                match self.cp.ask(&self.session, command, params) {
                    Answer::Value(Value::String(output))
                        if command == "platform_command_surface" =>
                    {
                        ToolExecutionResult::success(Value::String(output))
                    }
                    Answer::Value(value) => text(value),
                    Answer::Error { message, .. } => ToolExecutionResult::tool_error(message),
                }
            }
            PERSON_MCP_STORE_TOOL | PERSON_MCP_SERVERS_TOOL => {
                let command = if self.tool == PERSON_MCP_STORE_TOOL {
                    "user_mcp_store"
                } else {
                    "user_mcp_servers"
                };
                match self.cp.ask(&self.session, command, arguments) {
                    Answer::Value(value) => text(value),
                    Answer::Error { message, .. } => ToolExecutionResult::tool_error(message),
                }
            }
            other => ToolExecutionResult::tool_error(format!("no tool {other}")),
        }
    }
}
