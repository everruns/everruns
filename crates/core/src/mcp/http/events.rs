//! Events discovery gates every operation; schemas are refreshed before use so
//! routing annotations cannot silently lag a server's current descriptor.
use super::*;

pub(super) const EXTENSION: &str = "io.modelcontextprotocol/events";

#[allow(clippy::too_many_arguments)]
pub(super) async fn request(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    mode: McpProtocolMode,
    method: &str,
    params: &Value,
) -> Result<Value> {
    // Routing belongs to this operation, never to static connection headers.
    let clean_headers: HashMap<_, _> = headers
        .iter()
        .filter(|(name, _)| {
            let name = name.to_ascii_lowercase();
            !matches!(
                name.as_str(),
                "mcp-name" | "mcp-method" | "mcp-protocol-version" | "mcp-session-id"
            ) && !name.starts_with("mcp-param-")
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let headers = &clean_headers;
    let capabilities = ClientCapabilities::none();
    let (text, negotiated) = negotiate_and_send(
        egress,
        url,
        headers,
        credential,
        mode,
        "server/discover",
        None,
        &|version| {
            protocol::request_body(
                1,
                "server/discover",
                &serde_json::json!({}),
                version,
                capabilities,
            )
        },
        None,
        CALL_TIMEOUT,
        false,
    )
    .await?;
    let caps = if negotiated.stateful {
        negotiated
            .server_capabilities_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .unwrap_or_default()
    } else {
        parse_rpc_result(&text)?
            .get("capabilities")
            .cloned()
            .unwrap_or_default()
    };
    if !caps
        .get("extensions")
        .and_then(|e| e.get(EXTENSION))
        .is_some_and(Value::is_object)
    {
        return Err(McpRpcError {
            code: -32601,
            message: "Server did not advertise the Events extension".into(),
            data: None,
        }
        .into());
    }
    let mut listed = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let mut list_params = serde_json::json!({});
        if let Some(cursor) = &cursor {
            list_params["cursor"] = Value::String(cursor.clone());
        }
        let (text, _) = negotiate_and_send(
            egress,
            url,
            headers,
            credential,
            mode,
            "events/list",
            None,
            &|version| {
                protocol::request_body(2, "events/list", &list_params, version, capabilities)
            },
            Some(negotiated.clone()),
            CALL_TIMEOUT,
            false,
        )
        .await?;
        let page = parse_rpc_result(&text)?;
        for descriptor in page
            .get("events")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Invalid events/list response"))?
        {
            match param_headers::header_params(&descriptor["inputSchema"]) {
                Ok(_) => listed.push(descriptor.clone()),
                Err(reason) => {
                    tracing::warn!(%reason, "Excluded event with invalid header annotations")
                }
            }
        }
        cursor = page
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_string);
        if listed.len() > 10_000 {
            return Err(anyhow!("events/list descriptor limit exceeded"));
        }
        let Some(next) = &cursor else {
            break;
        };
        if !seen.insert(next.clone()) || seen.len() >= 32 {
            return Err(anyhow!("events/list pagination limit exceeded"));
        }
    }
    if method == "events/list" {
        return Ok(serde_json::json!({"resultType":"complete", "events":listed}));
    }
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Events request requires name"))?;
    let descriptor = listed
        .iter()
        .find(|d| d["name"] == name)
        .ok_or_else(|| anyhow!("Event is not available: {name}"))?;
    if method == "events/subscribe"
        && !descriptor
            .get("delivery")
            .and_then(Value::as_array)
            .is_some_and(|modes| modes.iter().any(|mode| mode == "webhook"))
    {
        return Err(anyhow!("Event does not offer webhook delivery: {name}"));
    }
    let annotations =
        param_headers::header_params(&descriptor["inputSchema"]).map_err(anyhow::Error::msg)?;
    let mut event_headers = headers.clone();
    for (key, value) in param_headers::headers_for(&annotations, &params["arguments"])
        .map_err(anyhow::Error::msg)?
    {
        event_headers.insert(key, value);
    }
    let (text, _) = negotiate_and_send(
        egress,
        url,
        &event_headers,
        credential,
        mode,
        method,
        Some(name),
        &|version| protocol::request_body(3, method, params, version, capabilities),
        Some(negotiated),
        CALL_TIMEOUT,
        false,
    )
    .await?;
    parse_rpc_result(&text)
}
