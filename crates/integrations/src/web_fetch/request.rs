//! Raw HTTP requests through `web_fetch`: any method, custom headers, a body.
//!
//! FetchKit only issues GET and HEAD and owns the request headers, so it cannot
//! follow a service's own instructions for agents (auth.md registration, OAuth
//! token polling, calling an API with a bearer token). A call that names another
//! method, or passes `headers`, `body`, `json` or `form`, is sent here instead:
//! straight through the host egress boundary as one request, with the response
//! returned as status, headers and text.
//!
//! Decision: the same tool, not a second one. The model already reaches for
//! `web_fetch` for "talk to this URL", the egress policy is identical, and
//! `web_fetch` is open-world, so tool approval already treats it as the
//! riskiest class whatever the method.
//!
//! THREAT[TM-AGENT-018]: the system allowlist and the session access list are
//! checked before sending and again at the egress boundary; DNS pinning blocks
//! private and link-local targets (TM-TOOL-018); redirects are returned, never
//! followed, so a 3xx cannot carry a body or credentials to another host.

use super::egress::{EgressError, EgressRequest, EgressRequestKind};
use super::tool_context::ToolContext;
use super::tools::ToolExecutionResult;
use everruns_contracts::runtime::{EgressAccess, SystemEgressPolicy};
use serde_json::{Map, Value, json};

/// Shared by both paths so the model sees one list of methods.
pub(super) const INVALID_METHOD: &str =
    "Invalid method: must be GET, HEAD, POST, PUT, PATCH, DELETE or OPTIONS";
const RAW_METHODS: [&str; 5] = ["POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
const BODY_FIELDS: [&str; 3] = ["body", "json", "form"];
const MAX_HEADERS: usize = 32;
const MAX_REQUEST_BODY: usize = 256 * 1024;
const MAX_RESPONSE_BODY: usize = 256 * 1024;
const TIMEOUT_MS: u64 = 30_000;
/// Headers the transport owns; a caller setting them would desync framing or
/// retarget the request.
const RESERVED_HEADERS: [&str; 6] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
];

/// Whether these arguments need the raw path rather than FetchKit.
pub(super) fn is_raw(arguments: &Value) -> bool {
    let method_is_raw = arguments
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|m| !m.eq_ignore_ascii_case("GET") && !m.eq_ignore_ascii_case("HEAD"));
    method_is_raw
        || arguments.get("headers").is_some_and(|v| !v.is_null())
        || BODY_FIELDS
            .iter()
            .any(|field| arguments.get(*field).is_some_and(|v| !v.is_null()))
}

/// Add the raw-request fields to FetchKit's schema.
pub(super) fn extend_schema(mut schema: Value) -> Value {
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };
    properties.insert(
        "method".to_string(),
        json!({
            "type": "string",
            "enum": ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"],
            "default": "GET",
            "description": "HTTP method. Defaults to GET. Methods other than GET and HEAD, or any \
                            of headers/body/json/form, send one raw request and return status, \
                            headers and the response text unconverted; redirects are returned, \
                            not followed."
        }),
    );
    properties.insert(
        "headers".to_string(),
        json!({
            "type": "object",
            "additionalProperties": {"type": "string"},
            "description": "Request headers, e.g. {\"Authorization\": \"Bearer ...\"}."
        }),
    );
    properties.insert(
        "body".to_string(),
        json!({"type": "string", "description": "Raw request body. Set Content-Type in headers."}),
    );
    properties.insert(
        "json".to_string(),
        json!({"description": "JSON request body; sets Content-Type: application/json."}),
    );
    properties.insert(
        "form".to_string(),
        json!({
            "type": "object",
            "additionalProperties": {"type": ["string", "number", "boolean"]},
            "description": "Form fields sent as application/x-www-form-urlencoded."
        }),
    );
    schema
}

/// Appended to FetchKit's tool description.
pub(super) const DESCRIPTION_SUFFIX: &str = "\n\nAlso sends API requests: set method \
(POST, PUT, PATCH, DELETE), headers, and one of body, json or form. Use this to follow a \
service's own instructions for agents, such as an auth.md registration or an OAuth token \
request. Treat returned credentials as secrets: do not repeat them to the user.";

struct RawRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

fn parse(arguments: &Value) -> Result<RawRequest, String> {
    let url = arguments
        .get("url")
        .and_then(Value::as_str)
        .ok_or("Missing required parameter: url")?
        .to_string();
    let parsed = url::Url::parse(&url).map_err(|_| "Invalid URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Invalid URL: must start with http:// or https://".to_string());
    }

    let method = match arguments.get("method") {
        None | Some(Value::Null) => "GET".to_string(),
        Some(Value::String(m)) => {
            let m = m.to_ascii_uppercase();
            if m != "GET" && m != "HEAD" && !RAW_METHODS.contains(&m.as_str()) {
                return Err(INVALID_METHOD.to_string());
            }
            m
        }
        Some(_) => return Err(INVALID_METHOD.to_string()),
    };

    let mut headers = Vec::new();
    match arguments.get("headers") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            if map.len() > MAX_HEADERS {
                return Err(format!("Too many headers: at most {MAX_HEADERS}"));
            }
            for (name, value) in map {
                let Value::String(value) = value else {
                    return Err(format!("Header {name} must be a string"));
                };
                if RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                    return Err(format!("Header {name} is set by the platform"));
                }
                if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
                    return Err(format!("Header {name} contains invalid characters"));
                }
                headers.push((name.clone(), value.clone()));
            }
        }
        Some(_) => return Err("headers must be an object of strings".to_string()),
    }

    let present: Vec<&str> = BODY_FIELDS
        .iter()
        .copied()
        .filter(|f| arguments.get(*f).is_some_and(|v| !v.is_null()))
        .collect();
    if present.len() > 1 {
        return Err("Pass only one of body, json or form".to_string());
    }
    let (body, content_type) = match present.first().copied() {
        None => (None, None),
        Some("body") => match arguments.get("body") {
            Some(Value::String(s)) => (Some(s.clone().into_bytes()), None),
            _ => return Err("body must be a string".to_string()),
        },
        Some("json") => {
            let bytes = serde_json::to_vec(&arguments["json"]).map_err(|e| e.to_string())?;
            (Some(bytes), Some("application/json"))
        }
        Some(_) => {
            let Some(fields) = arguments.get("form").and_then(Value::as_object) else {
                return Err("form must be an object".to_string());
            };
            (
                Some(encode_form(fields)?.into_bytes()),
                Some("application/x-www-form-urlencoded"),
            )
        }
    };
    if body.as_ref().is_some_and(|b| b.len() > MAX_REQUEST_BODY) {
        return Err(format!("Request body exceeds {MAX_REQUEST_BODY} bytes"));
    }
    if body.is_some() && (method == "GET" || method == "HEAD") {
        return Err(format!("{method} requests cannot carry a body"));
    }
    if let Some(content_type) = content_type
        && !headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("Content-Type".to_string(), content_type.to_string()));
    }

    Ok(RawRequest {
        method,
        url,
        headers,
        body,
    })
}

fn encode_form(fields: &Map<String, Value>) -> Result<String, String> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in fields {
        let value = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => {
                return Err(format!(
                    "form field {name} must be a string, number or boolean"
                ));
            }
        };
        serializer.append_pair(name, &value);
    }
    Ok(serializer.finish())
}

/// Send a raw request. Only called when [`is_raw`] is true.
pub(super) async fn execute(
    arguments: &Value,
    context: Option<&ToolContext>,
    system_policy: Option<&SystemEgressPolicy>,
) -> ToolExecutionResult {
    let request = match parse(arguments) {
        Ok(request) => request,
        Err(message) => return ToolExecutionResult::tool_error(message),
    };
    if let Some(policy) = system_policy {
        let access = EgressAccess::classify(&request.method, request.body.is_some());
        if let Err(denial) = policy.check(&request.url, access, None) {
            return ToolExecutionResult::tool_error(denial.message(&request.url));
        }
    }
    let network_access = context.and_then(|c| c.network_access.clone());
    if let Some(acl) = &network_access
        && !acl.is_url_allowed(&request.url)
    {
        return ToolExecutionResult::tool_error(format!(
            "URL blocked by network access policy: {}",
            request.url
        ));
    }
    let Some(egress) = context.and_then(|c| c.egress_service.clone()) else {
        return ToolExecutionResult::tool_error(
            "Requests with a method other than GET/HEAD, custom headers or a body need the \
             platform egress service, which this host does not provide",
        );
    };

    let mut egress_request =
        EgressRequest::new(&request.method, &request.url, EgressRequestKind::Capability)
            .network_access(network_access)
            .require_dns_pinning()
            .timeout_ms(TIMEOUT_MS);
    for (name, value) in request.headers {
        egress_request = egress_request.header(name, value);
    }
    if let Some(body) = request.body {
        egress_request = egress_request.body(body);
    }

    let response = match egress.send(egress_request).await {
        Ok(response) => response,
        Err(EgressError::NetworkAccessDenied { url }) => {
            return ToolExecutionResult::tool_error(format!(
                "Outbound request blocked by network policy: {url}"
            ));
        }
        Err(error) => return ToolExecutionResult::tool_error(format!("Request failed: {error}")),
    };

    let total = response.body.len();
    let truncated = total > MAX_RESPONSE_BODY;
    let body = &response.body[..total.min(MAX_RESPONSE_BODY)];
    let text = match std::str::from_utf8(body) {
        Ok(text) => Value::String(text.to_string()),
        // A cut can land inside a multi-byte character; keep what decodes.
        Err(error) if truncated && error.error_len().is_none() => {
            Value::String(String::from_utf8_lossy(&body[..error.valid_up_to()]).into_owned())
        }
        Err(_) => Value::Null,
    };
    let mut result = json!({
        "url": request.url,
        "method": request.method,
        "status_code": response.status,
        "headers": response.headers,
        "body": text,
        "size": total,
    });
    if truncated {
        result["truncated"] = Value::Bool(true);
    }
    if result["body"].is_null() && total > 0 {
        result["error"] = Value::String("Binary response body omitted".to_string());
    }
    ToolExecutionResult::success(result)
}

#[cfg(test)]
mod tests {
    use super::super::egress::{EgressResponse, EgressResult, EgressService, EgressStreamResponse};
    use super::super::typed_id::SessionId;
    use super::*;
    use async_trait::async_trait;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<EgressRequest>>,
        status: u16,
        body: Vec<u8>,
    }

    #[async_trait]
    impl EgressService for Recorder {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            self.seen.lock().unwrap().push(request);
            Ok(EgressResponse {
                status: self.status,
                headers: BTreeMap::from([(
                    "www-authenticate".to_string(),
                    "Bearer resource_metadata=\"x\"".to_string(),
                )]),
                body: self.body.clone(),
            })
        }

        async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            unreachable!("raw requests are not streamed")
        }
    }

    fn outcome(result: ToolExecutionResult) -> Value {
        match result {
            ToolExecutionResult::Success(value) => value,
            ToolExecutionResult::ToolError(error) => json!({ "error": error }),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    fn context(egress: Arc<Recorder>) -> ToolContext {
        let mut context = ToolContext::new(SessionId::new());
        context.egress_service = Some(egress);
        context
    }

    #[test]
    fn raw_path_only_for_methods_headers_or_bodies() {
        assert!(!is_raw(&json!({"url": "https://a.example"})));
        assert!(!is_raw(
            &json!({"url": "https://a.example", "method": "head"})
        ));
        assert!(is_raw(
            &json!({"url": "https://a.example", "method": "post"})
        ));
        assert!(is_raw(
            &json!({"url": "https://a.example", "headers": {"A": "b"}})
        ));
        assert!(is_raw(
            &json!({"url": "https://a.example", "form": {"a": "b"}})
        ));
        assert!(!is_raw(&json!({"url": "https://a.example", "json": null})));
    }

    #[tokio::test]
    async fn posts_auth_md_registration_as_json() {
        let egress = Arc::new(Recorder {
            status: 200,
            body: br#"{"claim_token":"clm_1"}"#.to_vec(),
            ..Default::default()
        });
        let result = outcome(
            execute(
                &json!({
                    "url": "https://svc.example/api/agent/identity",
                    "method": "POST",
                    "json": {"type": "service_auth", "login_hint": "a@example.com"}
                }),
                Some(&context(egress.clone())),
                None,
            )
            .await,
        );
        let value = result;
        assert_eq!(value["status_code"], 200);
        assert_eq!(value["body"], r#"{"claim_token":"clm_1"}"#);
        assert!(value["headers"]["www-authenticate"].is_string());

        let seen = egress.seen.lock().unwrap();
        assert_eq!(seen[0].method, "POST");
        assert!(seen[0].dns_pinning_required);
        assert_eq!(seen[0].headers["Content-Type"], "application/json");
        let body: Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(body["login_hint"], "a@example.com");
    }

    #[tokio::test]
    async fn polls_token_endpoint_as_form() {
        let egress = Arc::new(Recorder {
            status: 400,
            body: br#"{"error":"authorization_pending"}"#.to_vec(),
            ..Default::default()
        });
        let result = outcome(execute(
            &json!({
                "url": "https://svc.example/oauth2/token",
                "method": "post",
                "form": {"grant_type": "urn:workos:agent-auth:grant-type:claim", "claim_token": "c&1"}
            }),
            Some(&context(egress.clone())),
            None,
        )
        .await);
        // A 4xx is a result the agent acts on, not a tool failure.
        assert_eq!(result["status_code"], 400);
        let seen = egress.seen.lock().unwrap();
        assert_eq!(
            seen[0].headers["Content-Type"],
            "application/x-www-form-urlencoded"
        );
        assert_eq!(
            String::from_utf8(seen[0].body.clone()).unwrap(),
            "claim_token=c%261&grant_type=urn%3Aworkos%3Aagent-auth%3Agrant-type%3Aclaim"
        );
    }

    #[tokio::test]
    async fn get_with_bearer_header_uses_raw_path() {
        let egress = Arc::new(Recorder {
            status: 200,
            body: b"[]".to_vec(),
            ..Default::default()
        });
        let result = outcome(
            execute(
                &json!({
                    "url": "https://svc.example/api/v1/domains",
                    "headers": {"Authorization": "Bearer t"}
                }),
                Some(&context(egress.clone())),
                None,
            )
            .await,
        );
        assert_eq!(result["status_code"], 200);
        let seen = egress.seen.lock().unwrap();
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].headers["Authorization"], "Bearer t");
    }

    #[tokio::test]
    async fn rejects_bad_requests_before_sending() {
        let egress = Arc::new(Recorder::default());
        let ctx = context(egress.clone());
        for (args, needle) in [
            (
                json!({"url": "file:///etc/passwd", "method": "POST"}),
                "http",
            ),
            (
                json!({"url": "https://a.example", "method": "TRACE"}),
                "Invalid method",
            ),
            (
                json!({"url": "https://a.example", "method": "POST", "body": "x", "json": {}}),
                "only one",
            ),
            (
                json!({"url": "https://a.example", "body": "x"}),
                "cannot carry a body",
            ),
            (
                json!({"url": "https://a.example", "headers": {"Host": "evil"}}),
                "set by the platform",
            ),
            (
                json!({"url": "https://a.example", "headers": {"X": "a\r\nB: c"}}),
                "invalid characters",
            ),
            (
                json!({"url": "https://a.example", "method": "POST", "form": {"a": {"b": 1}}}),
                "form field",
            ),
        ] {
            let value = outcome(execute(&args, Some(&ctx), None).await);
            let error = value.to_string();
            assert!(error.contains(needle), "{args}: {error}");
        }
        assert!(egress.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn system_allowlist_and_missing_egress_block() {
        let allowlist = SystemEgressPolicy::allowlist_only(std::sync::Arc::new(
            everruns_contracts::runtime::SystemAllowlist::from_toml(
                "[groups.only]\nallowed = [\"allowed.example\"]\n",
            )
            .unwrap(),
        ));
        let egress = Arc::new(Recorder::default());
        let value = outcome(
            execute(
                &json!({"url": "https://other.example/x", "method": "POST"}),
                Some(&context(egress.clone())),
                Some(&allowlist),
            )
            .await,
        );
        assert!(value.to_string().contains("blocked by system policy"));
        assert!(egress.seen.lock().unwrap().is_empty());

        let value = outcome(
            execute(
                &json!({"url": "https://a.example", "method": "POST"}),
                None,
                None,
            )
            .await,
        );
        assert!(value.to_string().contains("egress service"));
    }

    #[tokio::test]
    async fn curated_writes_gates_writes_and_lets_reads_reach_egress() {
        let policy = SystemEgressPolicy::embedded(
            everruns_contracts::runtime::EgressPolicyMode::CuratedWrites,
        );
        let egress = Arc::new(Recorder::default());
        let value = outcome(
            execute(
                &json!({"url": "https://blog.example.net/x", "method": "POST", "body": "hi"}),
                Some(&context(egress.clone())),
                Some(&policy),
            )
            .await,
        );
        assert!(
            value.to_string().contains("requests that send data"),
            "{value}"
        );
        assert!(egress.seen.lock().unwrap().is_empty());

        // A read with custom headers is still a read: it reaches the boundary.
        let _ = execute(
            &json!({"url": "https://blog.example.net/x", "method": "GET", "headers": {"Accept": "text/plain"}}),
            Some(&context(egress.clone())),
            Some(&policy),
        )
        .await;
        assert_eq!(egress.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn schema_gains_raw_fields() {
        let schema = extend_schema(json!({"type": "object", "properties": {"url": {}}}));
        for field in ["method", "headers", "body", "json", "form"] {
            assert!(schema["properties"][field].is_object(), "{field}");
        }
        assert!(
            schema["properties"]["method"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("POST"))
        );
    }
}
