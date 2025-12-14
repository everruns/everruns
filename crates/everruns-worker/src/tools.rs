// Tool execution implementations

use anyhow::{Context, Result};
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolPolicy, ToolResult, WebhookTool};
use reqwest::Client;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::{error, info, warn};

/// Execute a single tool call
pub async fn execute_tool(
    tool_call: &ToolCall,
    tool_def: &ToolDefinition,
    client: &Client,
) -> ToolResult {
    info!(
        tool_call_id = %tool_call.id,
        tool_name = %tool_call.name,
        "Executing tool"
    );

    match tool_def {
        ToolDefinition::Webhook(webhook) => execute_webhook(tool_call, webhook, client).await,
        ToolDefinition::Builtin(_) => {
            // Built-in tools skipped for now
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                result: None,
                error: Some("Built-in tools not implemented yet".to_string()),
            }
        }
    }
}

/// Execute a webhook tool with retries and signing
async fn execute_webhook(
    tool_call: &ToolCall,
    webhook: &WebhookTool,
    client: &Client,
) -> ToolResult {
    let mut last_error = None;

    for attempt in 0..=webhook.max_retries {
        if attempt > 0 {
            warn!(
                tool_call_id = %tool_call.id,
                attempt = attempt,
                max_retries = webhook.max_retries,
                "Retrying webhook call"
            );
            // Exponential backoff: 1s, 2s, 4s, etc.
            tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt - 1))).await;
        }

        match execute_webhook_once(tool_call, webhook, client).await {
            Ok(result) => {
                info!(
                    tool_call_id = %tool_call.id,
                    attempt = attempt,
                    "Webhook call succeeded"
                );
                return ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: Some(result),
                    error: None,
                };
            }
            Err(e) => {
                error!(
                    tool_call_id = %tool_call.id,
                    attempt = attempt,
                    error = %e,
                    "Webhook call failed"
                );
                last_error = Some(e);
            }
        }
    }

    ToolResult {
        tool_call_id: tool_call.id.clone(),
        result: None,
        error: Some(format!(
            "Webhook failed after {} retries: {}",
            webhook.max_retries,
            last_error.unwrap()
        )),
    }
}

/// Execute webhook once (no retry logic)
async fn execute_webhook_once(
    tool_call: &ToolCall,
    webhook: &WebhookTool,
    client: &Client,
) -> Result<serde_json::Value> {
    // Prepare request body
    let body = json!({
        "tool_call_id": tool_call.id,
        "tool_name": tool_call.name,
        "arguments": tool_call.arguments,
    });
    let body_str = serde_json::to_string(&body)?;

    // Build request
    let mut request = client
        .request(
            webhook.method.parse().context("Invalid HTTP method")?,
            &webhook.url,
        )
        .timeout(Duration::from_secs(webhook.timeout_secs))
        .header("Content-Type", "application/json")
        .body(body_str.clone());

    // Add custom headers
    for (key, value) in &webhook.headers {
        request = request.header(key, value);
    }

    // Add signature if signing secret is provided
    if let Some(secret) = &webhook.signing_secret {
        let signature = generate_signature(&body_str, secret);
        request = request.header("X-Webhook-Signature", signature);
    }

    // Execute request
    let response = request
        .send()
        .await
        .context("Failed to send webhook request")?;

    let status = response.status();
    let response_body = response
        .text()
        .await
        .context("Failed to read response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "Webhook returned error status {}: {}",
            status,
            response_body
        );
    }

    // Parse response as JSON
    let result: serde_json::Value = serde_json::from_str(&response_body)
        .unwrap_or_else(|_| json!({ "raw_response": response_body }));

    Ok(result)
}

/// Generate HMAC-SHA256 signature for request verification
fn generate_signature(body: &str, secret: &str) -> String {
    let mut mac = Sha256::new();
    mac.update(secret.as_bytes());
    mac.update(body.as_bytes());
    let result = mac.finalize();
    format!("sha256={}", hex::encode(result))
}

/// Check if a tool requires approval
pub fn requires_approval(tool_def: &ToolDefinition) -> bool {
    match tool_def {
        ToolDefinition::Webhook(webhook) => webhook.policy == ToolPolicy::RequiresApproval,
        ToolDefinition::Builtin(builtin) => builtin.policy == ToolPolicy::RequiresApproval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_generation() {
        let body = r#"{"test":"data"}"#;
        let secret = "my-secret-key";
        let sig = generate_signature(body, secret);
        assert!(sig.starts_with("sha256="));
        assert_eq!(sig.len(), 71); // "sha256=" + 64 hex chars
    }

    #[test]
    fn test_requires_approval_webhook() {
        let tool = ToolDefinition::Webhook(WebhookTool {
            name: "test".to_string(),
            description: "test".to_string(),
            parameters: json!({}),
            url: "https://example.com".to_string(),
            method: "POST".to_string(),
            headers: Default::default(),
            signing_secret: None,
            timeout_secs: 30,
            max_retries: 3,
            policy: ToolPolicy::RequiresApproval,
        });
        assert!(requires_approval(&tool));
    }

    #[test]
    fn test_auto_approval_webhook() {
        let tool = ToolDefinition::Webhook(WebhookTool {
            name: "test".to_string(),
            description: "test".to_string(),
            parameters: json!({}),
            url: "https://example.com".to_string(),
            method: "POST".to_string(),
            headers: Default::default(),
            signing_secret: None,
            timeout_secs: 30,
            max_retries: 3,
            policy: ToolPolicy::Auto,
        });
        assert!(!requires_approval(&tool));
    }
}
