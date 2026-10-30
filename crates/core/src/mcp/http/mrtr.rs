//! Bounded multi-round-trip continuations for HTTP MCP operations.

use super::*;

/// Resource continuations run inline so dropping the caller cancels both input
/// collection and retries. One deadline covers negotiation and every round.
#[allow(clippy::too_many_arguments)]
pub(super) async fn request_with_inputs(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    mode: McpProtocolMode,
    server_name: &str,
    method: &str,
    params: &Value,
    cached: Option<Negotiated>,
    handlers: InputHandlers<'_>,
) -> Result<(String, Negotiated)> {
    let is_resource_read = method == "resources/read";
    // Discovery and unrelated methods still declare no inputs. prompts/get is
    // permitted by MCP but is not implemented by this client yet.
    let capabilities = ClientCapabilities {
        url_elicitation: is_resource_read && handlers.url.is_some(),
        form_elicitation: is_resource_read && handlers.form.is_some(),
    };
    tokio::time::timeout(CALL_TIMEOUT, async {
        let (text, negotiated) = negotiate_and_send(
            egress,
            url,
            headers,
            credential,
            mode,
            method,
            None,
            &|version| protocol::request_body(1, method, params, version, capabilities),
            cached,
            CALL_TIMEOUT,
        )
        .await?;
        let text = if is_resource_read {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("MCP resources/read requires a URI"))?;
            // Existing consent/answer stores use a string target. Include the
            // method and exact URI so reads of different resources cannot
            // consume each other's grants. Tool elicitations cannot use this
            // reserved prefix, even if a server ignores MCP naming guidance.
            let input_target = format!("resources/read:{uri}");
            resolve_input_required(
                egress,
                url,
                headers,
                credential,
                &negotiated,
                capabilities,
                server_name,
                MrtrRequest {
                    method,
                    name: None,
                    input_target: &input_target,
                    params,
                    idempotency_key: None,
                },
                handlers,
                CALL_TIMEOUT,
                text,
            )
            .await?
        } else {
            text
        };
        Ok((text, negotiated))
    })
    .await
    .map_err(|_| {
        anyhow!(
            "MCP {method} timed out after {} seconds",
            CALL_TIMEOUT.as_secs()
        )
    })?
}

/// The caller-owned operation. Never derive routing or consent scope from a
/// server's input requests or opaque continuation state.
pub(super) struct MrtrRequest<'a> {
    pub(super) method: &'a str,
    pub(super) name: Option<&'a str>,
    pub(super) input_target: &'a str,
    pub(super) params: &'a Value,
    pub(super) idempotency_key: Option<&'a str>,
}

/// Complete a multi round-trip tool call or resource read (MRTR).
///
/// A `2026-07-28` server may answer `tools/call` or `resources/read` with
/// `resultType: "input_required"` instead of a result. Three cases:
///
/// - **No `inputRequests`** — the server just needs the round trip (it stashed
///   context in `requestState`). Retry, echoing `requestState` under a new
///   JSON-RPC id, as the spec requires for an independent request.
/// - **A URL mode `elicitation/create`** — the server wants a human to complete
///   something out of band (third-party OAuth, a credential form, a payment).
///   Ask the host's handler for consent, then retry with the `accept` response
///   so the server can check whether the interaction completed. A decline or a
///   cancel ends the call: the client has nothing to send that would let the
///   server proceed, and retrying would only ask the same human again.
/// - **A form mode `elicitation/create`**, to a server whose policy opted in —
///   the server wants structured answers. The host's handler sends a recorded
///   answer, or stands the call down so a person can be asked
///   ([`crate::mcp::form_elicitation`]).
/// - **Anything else** — undeclared elicitation modes, sampling, or roots. A
///   server MUST NOT request an input type the client did not declare. Fail
///   naming the culprit rather than handing back an empty result the caller
///   would read as success.
///
/// Bounded by [`MAX_INPUT_REQUIRED_ROUNDS`]; a server that keeps asking gets a
/// clear error instead of an unbounded loop against the call timeout.
#[allow(clippy::too_many_arguments)]
pub(super) async fn resolve_input_required(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    negotiated: &Negotiated,
    capabilities: ClientCapabilities,
    server_name: &str,
    request: MrtrRequest<'_>,
    handlers: InputHandlers<'_>,
    call_timeout: Duration,
    text: String,
) -> Result<String> {
    let mut text = text;
    for round in 0..MAX_INPUT_REQUIRED_ROUNDS {
        let Some(json) = extract_json_from_response(&text) else {
            return Ok(text);
        };
        let Some(input_required) = protocol::input_required_from_result(json) else {
            return Ok(text);
        };

        // Consent stores predate resource reads and key on a string target.
        // THREAT[TM-TOOL-034]: fail closed for a tool impersonating a resource scope.
        if request.method == "tools/call" && request.input_target.starts_with("resources/read:") {
            return Err(anyhow!(
                "MCP tool name uses the reserved resource consent scope"
            ));
        }

        let input_responses = gather_input_responses(
            &input_required,
            server_name,
            request.input_target,
            handlers,
            url,
        )
        .await?;

        tracing::debug!(
            url = %url,
            method = %request.method,
            target = %request.input_target,
            round = round,
            inputs = input_responses.len(),
            "MCP request returned input_required; retrying"
        );
        // A distinct id per attempt: MRTR treats every retry as an independent
        // request.
        // The retry is the same call, so it carries the same key.
        let body = serde_json::to_vec(&protocol::with_idempotency_key(
            protocol::request_retry_body(
                round as i64 + 2,
                request.method,
                request.params,
                &negotiated.version,
                capabilities,
                input_required.request_state.as_deref(),
                &input_responses,
            ),
            request.idempotency_key,
        ))?;
        let response = send_op(
            egress,
            url,
            headers,
            credential,
            negotiated,
            request.method,
            request.name,
            body,
            call_timeout,
            None,
        )
        .await?;
        if !(200..300).contains(&response.status) {
            return Err(McpHttpStatusError {
                status: response.status,
                body: response.body,
            }
            .into());
        }
        text = response.body;
    }

    if extract_json_from_response(&text)
        .and_then(protocol::input_required_from_result)
        .is_some()
    {
        return Err(anyhow!(
            "MCP request '{}' for '{}' still requires input after {MAX_INPUT_REQUIRED_ROUNDS} \
             rounds; if you were asked to complete something in your browser, finish it and \
             retry the request",
            request.method,
            request.input_target,
        ));
    }
    Ok(text)
}
