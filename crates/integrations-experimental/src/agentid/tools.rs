//! `agentid_authorize`: approve an AgentID sign-in with the agent's inbox.
//!
//! Decisions:
//! - The key comes only from the agent's service-account `agentmail`
//!   connection (`get_service_api_key_connection`). Never the invoking end
//!   user's connection, never a management user's; no connection means the
//!   tool refuses.
//! - The request goes to one fixed AgentMail URL through the egress boundary
//!   with DNS pinning. The body carries the `auth_token` the agent was shown
//!   and `accept_disclosure: true`, nothing else (no mail send or read).
//! - AgentMail's 202 body (`api_key_id`, `instructions`) is returned as data.

use async_trait::async_trait;
use everruns_contracts::runtime::egress::{EgressError, EgressRequest, EgressRequestKind};
use everruns_contracts::runtime::tool_context::{ToolContext, ToolContextService};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::ToolHints;
use serde_json::{Value, json};

use super::connection::check_inbox_id;
use super::{AGENTMAIL_API_BASE, AGENTMAIL_PROVIDER};

const TIMEOUT_MS: u64 = 30_000;
const MAX_AUTH_TOKEN_LEN: usize = 4096;
const MAX_ERROR_CHARS: usize = 300;

/// Approve an AgentID sign-in for the agent's own AgentMail inbox.
pub struct AgentIdAuthorizeTool;

#[async_trait]
impl Tool for AgentIdAuthorizeTool {
    fn name(&self) -> &str {
        "agentid_authorize"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Sign in with AgentID")
    }

    fn description(&self) -> &str {
        "Approve an app's AgentID sign-in as this agent. Pass the auth_token shown on the app's \
         AgentID waiting page; the sign-in is approved for the agent's own AgentMail inbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "auth_token": {
                    "type": "string",
                    "description": "The auth token from the app's AgentID waiting page"
                }
            },
            "required": ["auth_token"],
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[
            ToolContextService::ConnectionResolver,
            ToolContextService::EgressService,
        ]
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("agentid_authorize requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let auth_token = match arguments.get("auth_token").and_then(Value::as_str) {
            Some(token) => token.trim(),
            None => return ToolExecutionResult::tool_error("auth_token is required."),
        };
        if auth_token.is_empty()
            || auth_token.len() > MAX_AUTH_TOKEN_LEN
            || auth_token.chars().any(char::is_control)
        {
            return ToolExecutionResult::tool_error("auth_token is not a valid AgentID token.");
        }
        let (Some(resolver), Some(egress)) =
            (&context.connection_resolver, &context.egress_service)
        else {
            return ToolExecutionResult::tool_error(
                "AgentID sign-in is not available in this runtime.",
            );
        };

        let connection = match resolver
            .get_service_api_key_connection(context.session_id, AGENTMAIL_PROVIDER)
            .await
        {
            Ok(Some(connection)) => connection,
            Ok(None) => {
                return ToolExecutionResult::tool_error(
                    "This agent has no AgentMail connection. An admin connects AgentMail to the \
                     agent's service account to enable AgentID sign-in.",
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "agentid_authorize: service connection lookup failed");
                return ToolExecutionResult::tool_error(
                    "Could not load the agent's AgentMail connection.",
                );
            }
        };
        let Some(inbox_id) = connection
            .metadata
            .as_ref()
            .and_then(|m| m.get("inbox_id"))
            .and_then(Value::as_str)
            .filter(|inbox| check_inbox_id(inbox).is_ok())
        else {
            return ToolExecutionResult::tool_error(
                "The agent's AgentMail connection has no inbox. Reconnect AgentMail with the inbox.",
            );
        };

        let url = authorize_url(inbox_id);
        let body = json!({ "auth_token": auth_token, "accept_disclosure": true });
        let request = EgressRequest::new("POST", url, EgressRequestKind::Integration)
            .network_access(context.network_access.clone())
            .require_dns_pinning()
            .timeout_ms(TIMEOUT_MS)
            .header("authorization", format!("Bearer {}", connection.api_key))
            .header("content-type", "application/json")
            .body(body.to_string());

        let response = match egress.send(request).await {
            Ok(response) => response,
            Err(EgressError::NetworkAccessDenied { .. }) => {
                return ToolExecutionResult::tool_error(
                    "AgentMail is blocked by this agent's network access policy.",
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "agentid_authorize: AgentMail request failed");
                return ToolExecutionResult::tool_error("Could not reach AgentMail.");
            }
        };
        let parsed: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        match response.status {
            200..=299 => ToolExecutionResult::success(json!({
                "authorized": true,
                "inbox_id": inbox_id,
                "api_key_id": parsed.get("api_key_id").cloned().unwrap_or(Value::Null),
                "instructions": parsed.get("instructions").cloned().unwrap_or(Value::Null),
            })),
            status => ToolExecutionResult::tool_error(error_message(status, &parsed)),
        }
    }
}

fn authorize_url(inbox_id: &str) -> String {
    format!(
        "{AGENTMAIL_API_BASE}/inboxes/{}/authorize",
        urlencoding::encode(inbox_id)
    )
}

fn error_message(status: u16, body: &Value) -> String {
    let field = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .map(|text| text.chars().take(MAX_ERROR_CHARS).collect::<String>())
    };
    let summary = match status {
        401 => "AgentMail rejected the agent's API key".to_string(),
        403 => "AgentMail refused the sign-in".to_string(),
        404 => "AgentMail did not find the sign-in or the agent's inbox".to_string(),
        400 => "AgentMail could not accept this auth token".to_string(),
        other => format!("AgentMail returned HTTP {other}"),
    };
    let detail = [field("code"), field("message"), field("fix")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(": ");
    if detail.is_empty() {
        format!("{summary}.")
    } else {
        format!("{summary}: {detail}")
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
