//! Generic JSON-RPC requests and response parsing.
use super::*;

/// Send one JSON-RPC request for any method (for example `events/subscribe`)
/// and return its `result`. Negotiates the protocol era (`Auto`) and applies
/// the same DNS-pinned SSRF contract as tool calls. A JSON-RPC error comes
/// back as an [`McpRpcError`] inside the `anyhow::Error`; a non-2xx HTTP
/// response as an [`McpHttpStatusError`].
pub async fn http_request(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    method: &str,
    params: &Value,
) -> Result<Value> {
    if method.starts_with("events/") {
        return tokio::time::timeout(
            CALL_TIMEOUT,
            events::request(
                egress,
                url,
                headers,
                credential,
                McpProtocolMode::Auto,
                method,
                params,
            ),
        )
        .await
        .map_err(|_| anyhow!("MCP Events request timed out"))?;
    }
    let (text, _) = request_with_inputs(
        egress,
        url,
        headers,
        credential,
        McpProtocolMode::Auto,
        url,
        method,
        params,
        None,
        InputHandlers {
            url: None,
            form: None,
            ui_apps: false,
        },
    )
    .await?;
    parse_rpc_result(&text)
}

/// Parse a JSON-RPC response body into its `result`, or an [`McpRpcError`].
pub(crate) fn parse_rpc_result(text: &str) -> Result<Value> {
    let json_str = extract_json_from_response(text)
        .ok_or_else(|| anyhow!("SSE response missing data line"))?;
    let mut response: Value = serde_json::from_str(json_str)?;
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(McpRpcError {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
            data: error.get("data").cloned(),
        }
        .into());
    }
    response
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| anyhow!("MCP server returned empty result"))
}
