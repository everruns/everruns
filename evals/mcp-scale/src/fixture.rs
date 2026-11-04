//! Synthetic MCP platform used by the study. The binary also runs as a
//! newline-delimited JSON-RPC stdio server, so Framework discovery, pagination,
//! dispatch, and state verification use the real production path.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const PAGE_SIZE: usize = 100;
const HTTP_BODY_LIMIT: usize = 32 * 1024 * 1024;
const HTTP_CATALOG_CACHE_ENTRIES: usize = 32;
const VIRTUAL_PUBLIC_MCP_AUTHORITY: &str = "93.184.215.14";

fn is_fixture_authority(authority: &str) -> bool {
    authority.eq_ignore_ascii_case("example.com") || authority == VIRTUAL_PUBLIC_MCP_AUTHORITY
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
}

fn string(description: &str, choices: &[&str]) -> Value {
    if choices.is_empty() {
        json!({"type":"string", "description":description})
    } else {
        json!({"type":"string", "description":description, "enum":choices})
    }
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({"name":name, "description":description, "inputSchema":schema})
}

pub fn catalog(size: usize) -> Vec<Value> {
    catalog_with_core(size, true)
}

fn catalog_with_core(size: usize, include_core: bool) -> Vec<Value> {
    let ticket = string("Ticket identifier from the ticket request.", &[]);
    let deployment = string(
        "Exact deployment identifier returned by list_deployments.",
        &[],
    );
    let mut tools = if include_core {
        vec![
            tool(
                "get_ticket",
                "Read a ticket, including its owner, status, and linked incident.",
                object(json!({"ticket_id":ticket}), &["ticket_id"]),
            ),
            tool(
                "search_tickets",
                "Find operational tickets by matching text.",
                object(json!({"query":string("Search terms.", &[])}), &["query"]),
            ),
            tool(
                "list_deployments",
                "Find deployments by service, environment, and status. Use this first to identify the exact deployment before inspection or change.",
                object(
                    json!({"service":string("Service slug.", &[]), "environment":string("Deployment environment.", &["dev", "staging", "prod"]), "status":string("Optional deployment state.", &["healthy", "failed", "rolling_back", "rolled_back"])}),
                    &["service", "environment"],
                ),
            ),
            tool(
                "get_deployment",
                "Read one deployment, including current version, previous version, health checks, and rollback eligibility.",
                object(
                    json!({"deployment_id":deployment.clone()}),
                    &["deployment_id"],
                ),
            ),
            tool(
                "rollback_deployment",
                "Roll a failed deployment back to its recorded previous version. Requires the exact deployment id and an explicit reason; this mutates deployment state.",
                object(
                    json!({"deployment_id":deployment, "reason":string("Operational reason for rollback.", &[])}),
                    &["deployment_id", "reason"],
                ),
            ),
            tool(
                "verify_deployment",
                "Verify deployment health and report the observed version and state.",
                object(
                    json!({"deployment_id":string("Deployment identifier.", &[])}),
                    &["deployment_id"],
                ),
            ),
            tool(
                "search_logs",
                "Search service logs by environment, query, and lookback range.",
                object(
                    json!({"service":string("Service slug.", &[]), "environment":string("Deployment environment.", &["dev", "staging", "prod"]), "query":string("Search text.", &[]), "time_range":string("Lookback interval.", &["15m", "1h", "6h", "24h"])}),
                    &["service", "environment", "query", "time_range"],
                ),
            ),
            tool(
                "get_incident",
                "Read an incident timeline, owner, impact, and linked ticket.",
                object(
                    json!({"incident_id":string("Incident identifier.", &[])}),
                    &["incident_id"],
                ),
            ),
        ]
    } else {
        // Every peer server exposes a plausible operation with the same API
        // name. The server description must disambiguate the telemetry source.
        vec![tool(
            "get_ticket",
            "Read a ticket, including its owner, status, and linked incident.",
            object(json!({"ticket_id":ticket}), &["ticket_id"]),
        )]
    };
    let domains = [
        "billing",
        "identity",
        "storage",
        "messaging",
        "search",
        "analytics",
        "inventory",
        "checkout",
        "notifications",
        "fraud",
        "reporting",
        "edge",
    ];
    let actions = [
        "inspect",
        "compare",
        "summarize",
        "validate",
        "lookup",
        "preview",
    ];
    let mut i = 0usize;
    while tools.len() < size {
        let domain = domains[i % domains.len()];
        let action = actions[(i / domains.len()) % actions.len()];
        let suffix = i / (domains.len() * actions.len()) + 1;
        let name = format!("{action}_{domain}_record_{suffix:03}");
        let schema = object(
            json!({"record_key":string("Synthetic record key.", &[]), "detail_level":string("Requested detail.", &["brief", "standard", "full"])}),
            &["record_key"],
        );
        tools.push(tool(
            &name,
            &format!("{action} a {domain} operation record by its exact key."),
            schema,
        ));
        i += 1;
    }
    tools.sort_by_key(|entry| fnv1a(entry["name"].as_str().unwrap_or_default().as_bytes()));
    tools
}

pub fn server_catalog(size: usize, include_core: bool) -> Vec<Value> {
    catalog_with_core(size, include_core)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

/// Serve the local fixture. `--size` controls the full catalog; `--server-id`
/// partitions state for multi-server trials; `--page-size` exposes pagination.
pub fn serve(args: &[String]) -> io::Result<()> {
    let value = |name: &str, fallback: usize| -> usize {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(fallback)
    };
    let size = value("--size", 600).clamp(1, 10_000);
    let page_size = value("--page-size", PAGE_SIZE).max(1);
    let server_id = args
        .iter()
        .position(|arg| arg == "--server-id")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "platform".into());
    let include_core = args.iter().any(|arg| arg == "--core");
    let tools = catalog_with_core(size, include_core);
    let state_file = args
        .iter()
        .position(|arg| arg == "--state-file")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let mut state = initial_state(&server_id);
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = request.get("id").cloned();
        let method = request["method"].as_str().unwrap_or_default();
        let params = &request["params"];
        let result = match method {
            "initialize" => Some(
                json!({"protocolVersion":"2025-06-18", "capabilities":{"tools":{"listChanged":false}}, "serverInfo":{"name":"everruns-mcp-scale-fixture", "version":"1.0.0"}}),
            ),
            "notifications/initialized" | "notifications/cancelled" => None,
            "tools/list" => {
                let offset = params["cursor"]
                    .as_str()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let end = (offset + page_size).min(tools.len());
                let page = tools[offset.min(tools.len())..end].to_vec();
                let next = (end < tools.len()).then(|| end.to_string());
                Some(json!({"tools":page, "nextCursor":next}))
            }
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let input = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let output = if let Some(path) = state_file.as_ref() {
                    match with_state_lock(path, &server_id, |state| {
                        let output = call(state, name, &input);
                        state["calls"].as_array_mut().unwrap().push(json!({"server":server_id, "tool":name, "arguments":input, "result":output.clone()}));
                        save_state(path, state)?;
                        Ok(output)
                    }) {
                        Ok(output) => output,
                        Err(error) => {
                            json!({"error":format!("fixture state unavailable: {error}")})
                        }
                    }
                } else {
                    let output = call(&mut state, name, &input);
                    state["calls"].as_array_mut().unwrap().push(json!({"server":server_id, "tool":name, "arguments":input, "result":output.clone()}));
                    output
                };
                let error = output.get("error").is_some();
                Some(
                    json!({"content":[{"type":"text", "text":output.to_string()}], "isError":error}),
                )
            }
            _ => {
                if let Some(id) = id.clone() {
                    writeln!(
                        stdout,
                        "{}",
                        json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601, "message":"Method not found"}})
                    )?;
                    stdout.flush()?;
                }
                None
            }
        };
        if let (Some(id), Some(result)) = (id, result) {
            writeln!(
                stdout,
                "{}",
                json!({"jsonrpc":"2.0", "id":id, "result":result})
            )?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Serve the same fixture over loopback Streamable HTTP MCP for hosted API/UI
/// acceptance cases. The control and audit routes are deliberately local and
/// unauthenticated; they let a human mutate synthetic fixture state without
/// changing agent decisions or introducing a Python gateway.
///
/// Routes:
/// - `POST /mcp/{fixture_id}/{server_id}`: MCP initialize/list/call.
/// - `GET|PUT /control/{fixture_id}[/{server_id}]`: read/patch fixture config.
/// - `GET /audit/{fixture_id}`: ordered listing and invocation evidence.
/// - `GET /healthz`: readiness.
pub fn serve_http(args: &[String]) -> io::Result<()> {
    let arg = |name: &str| {
        args.iter()
            .position(|item| item == name)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };
    let listen: SocketAddr = arg("--listen")
        .unwrap_or_else(|| "127.0.0.1:8765".into())
        .parse()
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    if !listen.ip().is_loopback() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "HTTP MCP fixture may bind only to a loopback address",
        ));
    }
    let listener = TcpListener::bind(listen)?;
    println!("READY http://{}/healthz", listener.local_addr()?);
    io::stdout().flush()?;
    let state = Arc::new(Mutex::new(HttpFixtureState::default()));
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    let _ = handle_http_connection(stream, state);
                });
            }
            Err(error) => eprintln!("HTTP fixture accept failed: {error}"),
        }
    }
    Ok(())
}

#[derive(Default)]
struct HttpFixtureState {
    config: std::collections::HashMap<String, Value>,
    server_config: std::collections::HashMap<(String, String), Value>,
    catalog_cache: std::collections::HashMap<(String, String), HttpCatalogCacheEntry>,
    fixture_state: std::collections::HashMap<(String, String), Value>,
    audit: std::collections::HashMap<String, Vec<Value>>,
    next_audit_id: u64,
}

struct HttpCatalogCacheEntry {
    config: Value,
    tools: Arc<Vec<Value>>,
}

impl HttpFixtureState {
    fn cached_catalog(
        &mut self,
        fixture_id: &str,
        server_id: &str,
        config: &Value,
    ) -> Arc<Vec<Value>> {
        let key = (fixture_id.to_owned(), server_id.to_owned());
        if let Some(entry) = self.catalog_cache.get(&key)
            && entry.config == *config
        {
            return Arc::clone(&entry.tools);
        }

        let tools = Arc::new(http_catalog(config, server_id));
        if self.catalog_cache.len() >= HTTP_CATALOG_CACHE_ENTRIES
            && let Some(oldest_key) = self.catalog_cache.keys().next().cloned()
        {
            self.catalog_cache.remove(&oldest_key);
        }
        self.catalog_cache.insert(
            key,
            HttpCatalogCacheEntry {
                config: config.clone(),
                tools: Arc::clone(&tools),
            },
        );
        tools
    }

    fn invalidate_catalog(&mut self, fixture_id: &str, server_id: Option<&str>) {
        if let Some(server_id) = server_id {
            self.catalog_cache
                .remove(&(fixture_id.to_owned(), server_id.to_owned()));
        } else {
            self.catalog_cache
                .retain(|(cached_fixture_id, _), _| cached_fixture_id != fixture_id);
        }
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: std::collections::HashMap<String, String>,
    body: Vec<u8>,
}

fn handle_http_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<HttpFixtureState>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let request = read_http_request(&mut stream)?;
    let (status, body, content_type) = route_http_request(&request, &state);
    write_http_response(&mut stream, status, content_type, &body)
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<HttpRequest> {
    let mut reader = io::BufReader::new(stream.try_clone()?);
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    if first_line.len() > 8192 || first_line.trim().is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    if method.is_empty() || path.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "malformed HTTP request line",
        ));
    }
    let mut headers = std::collections::HashMap::new();
    let mut total_header_bytes = first_line.len();
    loop {
        let mut line = String::new();
        let count = reader.read_line(&mut line)?;
        total_header_bytes += count;
        if total_header_bytes > 64 * 1024 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "HTTP headers too large",
            ));
        }
        if count == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "chunked request bodies are not supported by the local fixture",
        ));
    }
    let body_size = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if body_size > HTTP_BODY_LIMIT {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "HTTP body too large",
        ));
    }
    let mut body = vec![0; body_size];
    reader.read_exact(&mut body)?;
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Fixture Response",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn route_http_request(
    request: &HttpRequest,
    state: &Arc<Mutex<HttpFixtureState>>,
) -> (u16, Vec<u8>, &'static str) {
    let absolute_target =
        request.path.starts_with("http://") || request.path.starts_with("https://");
    let path = if absolute_target {
        let (scheme, rest) = match request.path.split_once("://") {
            Some(parts) => parts,
            None => return (400, b"invalid proxy request target".to_vec(), "text/plain"),
        };
        if scheme != "http" {
            return (403, b"fixture accepts HTTP only".to_vec(), "text/plain");
        }
        let (authority, uri_path) = rest.split_once('/').unwrap_or((rest, ""));
        if !is_fixture_authority(authority) {
            return (
                403,
                b"fixture rejects non-fixture proxy hosts".to_vec(),
                "text/plain",
            );
        }
        if request
            .headers
            .get("host")
            .is_some_and(|host| !host.eq_ignore_ascii_case(authority))
        {
            return (
                403,
                b"fixture authority and Host header disagree".to_vec(),
                "text/plain",
            );
        }
        format!("/{uri_path}")
    } else {
        request.path.clone()
    };
    let path = path.split('?').next().unwrap_or_default();
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let json_response = |status: u16, value: Value| {
        (
            status,
            serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
            "application/json; charset=utf-8",
        )
    };
    if path == "/healthz" && request.method == "GET" {
        return json_response(200, json!({"ready":true,"transport":"streamable-http-mcp"}));
    }
    if absolute_target && !path.starts_with("/mcp/") {
        return json_response(
            403,
            json!({"error":"forward proxy is restricted to fixture MCP endpoints"}),
        );
    }
    if !absolute_target && path.starts_with("/mcp/") {
        return json_response(
            403,
            json!({"error":"MCP requests must arrive through the tenant egress proxy"}),
        );
    }
    if parts.first() == Some(&"control") && parts.len() >= 2 {
        return route_fixture_control(request, &parts, state, json_response);
    }
    if parts.first() == Some(&"audit") && parts.len() == 2 && request.method == "GET" {
        let fixture_id = parts[1].to_string();
        let state = match state.lock() {
            Ok(state) => state,
            Err(_) => return json_response(500, json!({"error":"fixture state lock poisoned"})),
        };
        let entries = state.audit.get(&fixture_id).cloned().unwrap_or_default();
        return json_response(200, json!({"fixture_id":fixture_id,"entries":entries}));
    }
    if parts.first() == Some(&"mcp") && parts.len() == 3 {
        if request.method != "POST" {
            return json_response(405, json!({"error":"MCP fixture supports POST only"}));
        }
        let Some(proxy_authorization) = request.headers.get("proxy-authorization") else {
            return json_response(403, json!({"error":"missing tenant proxy authorization"}));
        };
        let Ok(secret) = std::env::var(everruns_core::host::EGRESS_PROXY_SECRET_ENV) else {
            return json_response(
                403,
                json!({"error":"tenant proxy verification is not configured"}),
            );
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if everruns_core::host::verify_proxy_authorization(
            Some(proxy_authorization),
            secret.as_bytes(),
            now,
        )
        .is_err()
        {
            return json_response(403, json!({"error":"invalid tenant proxy authorization"}));
        }
        let fixture_id = parts[1].to_string();
        let server_id = parts[2].to_string();
        let request_value = match serde_json::from_slice::<Value>(&request.body) {
            Ok(request) => request,
            Err(error) => {
                return json_response(400, json!({"error":format!("invalid MCP JSON: {error}")}));
            }
        };
        return route_mcp_request(
            &fixture_id,
            &server_id,
            &request_value,
            request,
            state,
            json_response,
        );
    }
    json_response(404, json!({"error":"fixture route not found"}))
}

fn route_fixture_control<F>(
    request: &HttpRequest,
    parts: &[&str],
    state: &Arc<Mutex<HttpFixtureState>>,
    response: F,
) -> (u16, Vec<u8>, &'static str)
where
    F: Fn(u16, Value) -> (u16, Vec<u8>, &'static str),
{
    if parts.len() > 3 || !["GET", "PUT"].contains(&request.method.as_str()) {
        return response(405, json!({"error":"control route supports GET and PUT"}));
    }
    let fixture_id = parts[1].to_string();
    let server_id = parts.get(2).map(|value| value.to_string());
    let mut state = match state.lock() {
        Ok(state) => state,
        Err(_) => return response(500, json!({"error":"fixture state lock poisoned"})),
    };
    if request.method == "PUT" {
        let patch = match serde_json::from_slice::<Value>(&request.body) {
            Ok(patch @ Value::Object(_)) => patch,
            Ok(_) => return response(400, json!({"error":"control body must be a JSON object"})),
            Err(error) => {
                return response(
                    400,
                    json!({"error":format!("invalid control JSON: {error}")}),
                );
            }
        };
        if let Some(server_id) = server_id.as_ref() {
            let key = (fixture_id.clone(), server_id.clone());
            let value = state
                .server_config
                .get(&key)
                .cloned()
                .unwrap_or_else(|| json!({}));
            state.server_config.insert(key, merge_json(value, &patch));
            state.invalidate_catalog(&fixture_id, Some(server_id));
        } else {
            let value = state
                .config
                .get(&fixture_id)
                .cloned()
                .unwrap_or_else(default_http_config);
            state
                .config
                .insert(fixture_id.clone(), merge_json(value, &patch));
            state.invalidate_catalog(&fixture_id, None);
        }
    }
    let config = match server_id {
        Some(server_id) => {
            let base = state
                .config
                .get(&fixture_id)
                .cloned()
                .unwrap_or_else(default_http_config);
            let override_config = state
                .server_config
                .get(&(fixture_id.clone(), server_id))
                .cloned()
                .unwrap_or_else(|| json!({}));
            merge_json(base, &override_config)
        }
        None => state
            .config
            .get(&fixture_id)
            .cloned()
            .unwrap_or_else(default_http_config),
    };
    response(200, config)
}

fn route_mcp_request<F>(
    fixture_id: &str,
    server_id: &str,
    request_value: &Value,
    request: &HttpRequest,
    state: &Arc<Mutex<HttpFixtureState>>,
    response: F,
) -> (u16, Vec<u8>, &'static str)
where
    F: Fn(u16, Value) -> (u16, Vec<u8>, &'static str),
{
    let id = request_value.get("id").cloned();
    let method = request_value["method"].as_str().unwrap_or_default();
    let params = &request_value["params"];
    let mut state = match state.lock() {
        Ok(state) => state,
        Err(_) => return response(500, json!({"error":"fixture state lock poisoned"})),
    };
    let config = {
        let base = state
            .config
            .get(fixture_id)
            .cloned()
            .unwrap_or_else(default_http_config);
        let override_config = state
            .server_config
            .get(&(fixture_id.to_owned(), server_id.to_owned()))
            .cloned()
            .unwrap_or_else(|| json!({}));
        merge_json(base, &override_config)
    };
    let audit_id = state.next_audit_id;
    state.next_audit_id += 1;
    let deny_status = config["deny"][method].as_u64().map(|status| status as u16);
    if let Some(status @ (401 | 403)) = deny_status {
        push_http_audit(
            &mut state,
            fixture_id,
            json!({
                "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"status":status,
                "schema_revision":config["schema_revision"],
                "request_content_type":request.headers.get("content-type")
            }),
        );
        return response(status, json!({"error":format!("fixture denied {method}")}));
    }
    if !config["attached"].as_bool().unwrap_or(true) || !server_authorized(&config, server_id) {
        push_http_audit(
            &mut state,
            fixture_id,
            json!({
                "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"status":404,
                "schema_revision":config["schema_revision"]
            }),
        );
        return response(404, json!({"error":"fixture server is detached"}));
    }
    let result = match method {
        "initialize" => Some(json!({
            "protocolVersion":"2025-06-18",
            "capabilities":{"tools":{"listChanged":false}},
            "serverInfo":{"name":format!("mcp-scale-{server_id}"),"version":config["schema_revision"]}
        })),
        "notifications/initialized" | "notifications/cancelled" => {
            push_http_audit(
                &mut state,
                fixture_id,
                json!({
                    "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"status":202,
                    "schema_revision":config["schema_revision"]
                }),
            );
            return (202, Vec::new(), "application/json; charset=utf-8");
        }
        "tools/list" => {
            let cursor = params["cursor"].as_str().unwrap_or_default().to_owned();
            let page_index = cursor.parse::<usize>().unwrap_or(0)
                / config["page_size"]
                    .as_u64()
                    .unwrap_or(PAGE_SIZE as u64)
                    .max(1) as usize;
            if config["fault"]["page"].as_u64() == Some(page_index as u64)
                && config["fault"]["kind"] == "fail_page"
            {
                push_http_audit(
                    &mut state,
                    fixture_id,
                    json!({
                        "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"cursor":cursor,
                        "page_index":page_index,"status":503,"schema_revision":config["schema_revision"]
                    }),
                );
                return response(503, json!({"error":"configured fixture page failure"}));
            }
            if config["fault"]["page"].as_u64() == Some(page_index as u64)
                && config["fault"]["kind"] == "delay_page"
            {
                thread::sleep(Duration::from_millis(
                    config["fault"]["delay_ms"]
                        .as_u64()
                        .unwrap_or(1_000)
                        .min(30_000),
                ));
            }
            let tools = state.cached_catalog(fixture_id, server_id, &config);
            let page_size = config["page_size"]
                .as_u64()
                .unwrap_or(PAGE_SIZE as u64)
                .max(1) as usize;
            let offset = cursor.parse::<usize>().unwrap_or(0).min(tools.len());
            let end = offset.saturating_add(page_size).min(tools.len());
            let mut page = tools[offset..end].to_vec();
            let next = (end < tools.len()).then(|| end.to_string());
            let mut next_cursor = next;
            if config["fault"]["page"].as_u64() == Some(page_index as u64) {
                match config["fault"]["kind"].as_str().unwrap_or_default() {
                    "repeated_cursor" => {
                        next_cursor = Some(if cursor.is_empty() {
                            "0".to_owned()
                        } else {
                            cursor.clone()
                        });
                    }
                    "empty_cursor" => next_cursor = Some(String::new()),
                    "duplicate_name" if !page.is_empty() => page.push(page[0].clone()),
                    _ => {}
                }
            }
            let names = page
                .iter()
                .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            let result = json!({"tools":page,"nextCursor":next_cursor.clone()});
            let result_bytes = serde_json::to_vec(&result)
                .map(|bytes| bytes.len())
                .unwrap_or(0);
            let envelope = json!({"jsonrpc":"2.0","id":id.clone(),"result":result});
            let response_bytes = serde_json::to_vec(&envelope)
                .map(|bytes| bytes.len())
                .unwrap_or(0);
            push_http_audit(
                &mut state,
                fixture_id,
                json!({
                    "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"cursor":cursor,
                    "page_index":page_index,"page_tool_names":names,"next_cursor":next_cursor,
                    "tool_count":names.len(),"schema_revision":config["schema_revision"],
                    "status":200,"result_bytes":result_bytes,"response_bytes":response_bytes
                }),
            );
            return response(200, envelope);
        }
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let key = (fixture_id.to_owned(), server_id.to_owned());
            let fixture_state = state
                .fixture_state
                .entry(key)
                .or_insert_with(|| initial_state(server_id));
            let output = dispatch_http_call(fixture_state, &config, server_id, name, &arguments);
            let is_error = output.get("error").is_some();
            let result = json!({
                "content":[{"type":"text","text":output.to_string()}],
                "isError":is_error
            });
            let result_bytes = serde_json::to_vec(&result)
                .map(|bytes| bytes.len())
                .unwrap_or(0);
            let envelope = json!({"jsonrpc":"2.0","id":id.clone(),"result":result});
            let response_bytes = serde_json::to_vec(&envelope)
                .map(|bytes| bytes.len())
                .unwrap_or(0);
            push_http_audit(
                &mut state,
                fixture_id,
                json!({
                    "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"tool":name,
                    "arguments":arguments,"schema_revision":config["schema_revision"],
                    "result":output,"result_bytes":result_bytes,"response_bytes":response_bytes,"status":200
                }),
            );
            return response(200, envelope);
        }
        _ => None,
    };
    if let (Some(id), Some(result)) = (id.clone(), result) {
        push_http_audit(
            &mut state,
            fixture_id,
            json!({
                "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"status":200,
                "schema_revision":config["schema_revision"]
            }),
        );
        response(200, json!({"jsonrpc":"2.0","id":id,"result":result}))
    } else if let Some(id) = id {
        push_http_audit(
            &mut state,
            fixture_id,
            json!({
                "id":id.clone(),"audit_id":audit_id,"server":server_id,"method":method,"status":200,
                "schema_revision":config["schema_revision"]
            }),
        );
        response(
            200,
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
        )
    } else {
        response(202, json!({}))
    }
}

fn push_http_audit(state: &mut HttpFixtureState, fixture_id: &str, entry: Value) {
    let audit = state.audit.entry(fixture_id.to_owned()).or_default();
    if audit.len() >= 100_000 {
        audit.remove(0);
    }
    audit.push(entry);
}

fn default_http_config() -> Value {
    json!({
        "scenario":"catalog",
        "catalog_size":600,
        "page_size":PAGE_SIZE,
        "schema_revision":"A",
        "target_tool":"get_ticket",
        "attached":true,
        "authorized_servers":null,
        "server_description":null,
        "catalog_json_bytes_target":null,
        "fault":null,
        "deny":{}
    })
}

fn merge_json(mut base: Value, patch: &Value) -> Value {
    if let (Value::Object(base), Value::Object(patch)) = (&mut base, patch) {
        for (key, value) in patch {
            if value.is_null() {
                base.remove(key);
            } else if value.is_object() && base.get(key).is_some_and(Value::is_object) {
                let merged = merge_json(base.get(key).cloned().unwrap(), value);
                base.insert(key.clone(), merged);
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    }
    base
}

fn http_catalog(config: &Value, server_id: &str) -> Vec<Value> {
    let scenario = config["scenario"].as_str().unwrap_or("catalog");
    if !config["attached"].as_bool().unwrap_or(true) || !server_authorized(config, server_id) {
        return Vec::new();
    }
    let description = config["server_description"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| match (scenario, server_id) {
            ("colliding-search", "alpha") => {
                "Geothermal drilling telemetry and field operations records".into()
            }
            ("colliding-search", "beta") => {
                "Urban water quality sampling and treatment records".into()
            }
            ("colliding-search", "hidden") => {
                "Private maintenance records unavailable to this agent".into()
            }
            _ => format!("Synthetic {server_id} MCP fixture server"),
        });
    let size = config["catalog_size"].as_u64().unwrap_or(600).min(20_000) as usize;
    let mut tools = match scenario {
        "colliding-search" => vec![colliding_search_tool(server_id)],
        "dynamic" => vec![dynamic_tool(config)],
        "deferred" => vec![tool(
            &format!("read_{}", sanitize_tool_fragment(server_id)),
            &format!("Read a harmless record from {description}."),
            object(
                json!({"filter":{"type":"object","description":"Domain-specific lookup criteria.","properties":{"key":{"type":"string","description":"The exact synthetic record key."}},"required":["key"],"additionalProperties":false}}),
                &["filter"],
            ),
        )],
        _ => {
            let target = config["target_tool"].as_str().unwrap_or("get_ticket");
            let mut tools = server_catalog(size.max(1), false);
            let target_tool = paginated_target_tool(target, config);
            tools.retain(|entry| entry["name"].as_str() != Some(target));
            if tools.len() >= size.max(1) {
                tools.pop();
            }
            tools.push(target_tool);
            tools
        }
    };
    if let Some(target_bytes) = config["catalog_json_bytes_target"].as_u64() {
        let current = serde_json::to_vec(&tools)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let padding = (target_bytes as usize).saturating_sub(current);
        if let Some(first) = tools.first_mut() {
            let old = first["description"].as_str().unwrap_or_default().to_owned();
            first["description"] = json!(format!("{old}{}", "x".repeat(padding)));
        }
    }
    if let Some(padding) = config["padding_bytes"].as_u64() {
        if let Some(first) = tools.first_mut() {
            let old = first["description"].as_str().unwrap_or_default().to_owned();
            first["description"] = json!(format!(
                "{old}{}",
                "x".repeat((padding as usize).min(HTTP_BODY_LIMIT))
            ));
        }
    }
    // Keep server description visible in protocol operation metadata for
    // deployments that cannot expose the host's MCP server description field.
    if config["include_server_description_on_tools"]
        .as_bool()
        .unwrap_or(false)
    {
        if let Some(first) = tools.first_mut() {
            let existing = first["description"].as_str().unwrap_or_default();
            first["description"] = json!(format!("{existing} Server context: {description}"));
        }
    }
    tools
}

fn server_authorized(config: &Value, server_id: &str) -> bool {
    if let Some(servers) = config["authorized_servers"].as_array() {
        return servers
            .iter()
            .any(|server| server.as_str() == Some(server_id));
    }
    !(config["scenario"] == "colliding-search" && server_id == "hidden")
}

fn paginated_target_tool(name: &str, config: &Value) -> Value {
    match name {
        "get_incident" => tool(
            name,
            "Read the incident record, owner, impact, and linked ticket using the nested incident selector.",
            object(
                json!({"filter":{"type":"object","description":"Incident selection criteria.","properties":{"incident_id":{"type":"string","description":"Exact incident identifier, for example INC-427."}},"required":["incident_id"],"additionalProperties":false},"include_timeline":{"type":"boolean","description":"Whether to include the concise event timeline."}}),
                &["filter"],
            ),
        ),
        "get_ticket" => tool(
            name,
            "Read the ticket owner, status, and linked incident using the nested ticket selector.",
            object(
                json!({"filter":{"type":"object","description":"Ticket selection criteria.","properties":{"ticket_id":{"type":"string","description":"Exact ticket identifier, for example OPS-8421."}},"required":["ticket_id"],"additionalProperties":false},"include_history":{"type":"boolean","description":"Whether to include the concise history."}}),
                &["filter"],
            ),
        ),
        _ => dynamic_tool(config),
    }
}

fn colliding_search_tool(server_id: &str) -> Value {
    let properties = match server_id {
        "alpha" => json!({
            "well_pad": {"type":"string", "description":"Synthetic well-pad identifier."},
            "rig_id": {"type":"string", "description":"Synthetic geothermal rig identifier."}
        }),
        "beta" => json!({
            "watershed_id": {"type":"string", "description":"Synthetic watershed identifier."},
            "sample_id": {"type":"string", "description":"Synthetic water-quality sample identifier."}
        }),
        _ => json!({"record_key":{"type":"string","description":"Synthetic private record key."}}),
    };
    let required = match server_id {
        "alpha" => vec!["well_pad"],
        "beta" => vec!["watershed_id"],
        _ => vec!["record_key"],
    };
    tool(
        "search",
        "Search records using the structured filter object.",
        object(
            json!({"filter":{"type":"object","description":"Domain-specific search criteria.","properties":properties,"required":required,"additionalProperties":false}}),
            &["filter"],
        ),
    )
}

fn dynamic_tool(config: &Value) -> Value {
    let revision = config["schema_revision"].as_str().unwrap_or("A");
    if revision == "B" || revision == "2" {
        tool(
            "read_item",
            "Read an item using the current versioned schema; revision B requires revision_token.",
            object(
                json!({"item":{"type":"object","properties":{"item_key":{"type":"string","description":"The synthetic item key."},"revision_token":{"type":"string","description":"Must equal the current schema revision token."}},"required":["item_key","revision_token"],"additionalProperties":false}}),
                &["item"],
            ),
        )
    } else {
        tool(
            "read_item",
            "Read an item using schema revision A and the nested item selector.",
            object(
                json!({"item":{"type":"object","properties":{"item_key":{"type":"string","description":"The synthetic item key."}},"required":["item_key"],"additionalProperties":false}}),
                &["item"],
            ),
        )
    }
}

fn dispatch_http_call(
    state: &mut Value,
    config: &Value,
    server_id: &str,
    name: &str,
    args: &Value,
) -> Value {
    if config["scenario"] == "colliding-search" && name == "search" {
        let field = match server_id {
            "alpha" => "well_pad",
            "beta" => "watershed_id",
            _ => "record_key",
        };
        let query = args["filter"][field].as_str().unwrap_or_default();
        let source = match server_id {
            "alpha" => "alpha",
            "beta" => "beta",
            _ => "hidden",
        };
        return json!({"source":source,"query":query,"record":{"record_id":format!("{}-record-001",source),"domain_result":source}});
    }
    if config["scenario"] == "dynamic" && name == "read_item" {
        if !state["server_id"].is_string() {
            *state = initial_state(server_id);
        }
        let revision = config["schema_revision"].as_str().unwrap_or("A");
        if revision == "B" || revision == "2" {
            if args["item"]["revision_token"].as_str() != Some(revision) {
                return json!({"error":"current schema revision token is required"});
            }
        }
        return json!({"item_key":args["item"]["item_key"],"schema_revision":revision,"server":server_id,"status":"found"});
    }
    if name == "get_ticket" {
        let ticket_id = args["filter"]["ticket_id"].as_str().unwrap_or_default();
        if ticket_id == "OPS-8421" && server_id == "telemetry" {
            return json!({"ticket_id":"OPS-8421","title":"Investigate billing-api staging 5xx","owner":"Ava Li","status":"in_progress","linked_incident":"INC-427","server":server_id,"schema_revision":config["schema_revision"]});
        }
        return json!({"error":"requested ticket is not present on this server"});
    }
    if name == "get_incident" {
        let incident_id = args["filter"]["incident_id"].as_str().unwrap_or_default();
        if incident_id == "INC-427" && server_id == "telemetry" {
            return json!({"incident_id":"INC-427","owner":"Ava Li","linked_ticket":"OPS-8421","status":"investigating","server":server_id,"schema_revision":config["schema_revision"]});
        }
        return json!({"error":"requested incident is not present on this server"});
    }
    call(state, name, args)
}

fn sanitize_tool_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn initial_state(server_id: &str) -> Value {
    json!({
        "ticket": {"ticket_id":"OPS-8421", "title":"Investigate billing-api staging 5xx", "owner":"Ava Li", "status":"in_progress", "linked_incident":"INC-427"},
        "staging": {"deployment_id":"dep-bill-stg-2042", "service":"billing-api", "environment":"staging", "version":"3.8.1", "previous_version":"3.8.0", "status":"failed", "rollback_eligible":true},
        "production": {"deployment_id":"dep-bill-prd-9911", "service":"billing-api", "environment":"prod", "version":"3.8.1", "previous_version":"3.8.0", "status":"healthy", "rollback_eligible":false},
        "server_id":server_id,
        "calls": []
    })
}

fn with_state_lock<T>(
    path: &PathBuf,
    server_id: &str,
    action: impl FnOnce(&mut Value) -> io::Result<T>,
) -> io::Result<T> {
    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_path = PathBuf::from(lock_name);
    let started = Instant::now();
    let lock = loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => break file,
            Err(error)
                if error.kind() == ErrorKind::AlreadyExists
                    && started.elapsed() < Duration::from_secs(10) =>
            {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => return Err(error),
        }
    };
    let mut state = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| initial_state(server_id)),
        Err(error) if error.kind() == ErrorKind::NotFound => initial_state(server_id),
        Err(error) => {
            let _ = fs::remove_file(&lock_path);
            return Err(error);
        }
    };
    let outcome = action(&mut state);
    drop(lock);
    let _ = fs::remove_file(lock_path);
    outcome
}

fn save_state(path: &PathBuf, state: &Value) -> io::Result<()> {
    let mut temp_name = path.as_os_str().to_os_string();
    temp_name.push(".tmp");
    let temp = PathBuf::from(temp_name);
    fs::write(&temp, serde_json::to_vec(state).map_err(io::Error::other)?)?;
    fs::rename(temp, path)
}

fn call(state: &mut Value, name: &str, args: &Value) -> Value {
    match name {
        "get_ticket" if args["ticket_id"] == "OPS-8421" && state["server_id"] == "telemetry" => {
            state["ticket"].clone()
        }
        "get_ticket" => json!({"error":"ticket OPS-8421 is not present on this server"}),
        "list_deployments" if args["service"] == "billing-api" => {
            let entries = [&state["staging"], &state["production"]]
                .into_iter()
                .filter(|deployment| {
                    args["environment"]
                        .as_str()
                        .is_none_or(|env| deployment["environment"] == env)
                })
                .filter(|deployment| {
                    args["status"]
                        .as_str()
                        .is_none_or(|status| deployment["status"] == status)
                })
                .cloned()
                .collect::<Vec<_>>();
            json!(entries)
        }
        "get_deployment" | "verify_deployment" => {
            deployment(state, args["deployment_id"].as_str().unwrap_or_default())
                .cloned()
                .unwrap_or_else(|| json!({"error":"deployment not found"}))
        }
        "rollback_deployment" => {
            if args["deployment_id"] != "dep-bill-stg-2042"
                || args["reason"].as_str().is_none_or(str::is_empty)
            {
                json!({"error":"rollback requires the exact failed staging deployment and a reason"})
            } else {
                state["staging"]["version"] = state["staging"]["previous_version"].clone();
                state["staging"]["status"] = json!("rolled_back");
                state["staging"]["rollback_eligible"] = json!(false);
                state["staging"].clone()
            }
        }
        "search_tickets"
            if args["query"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("ops-8421")
                || args["query"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains("billing") =>
        {
            json!([state["ticket"]])
        }
        "get_incident" if args["incident_id"] == "INC-427" => {
            json!({"incident_id":"INC-427", "owner":"Ava Li", "linked_ticket":"OPS-8421", "status":"investigating"})
        }
        "search_logs" => {
            json!([{"log_id":"log-8421", "deployment_id":"dep-bill-stg-2042", "severity":"error", "message":"HTTP 5xx increase after staging release"}])
        }
        _ if !name.contains("record_") => {
            json!({"error":format!("unknown tool or invalid arguments for {name}")})
        }
        _ => {
            json!({"record_key":args["record_key"], "detail":args["detail_level"], "server":state["server_id"]})
        }
    }
}

fn deployment<'a>(state: &'a Value, id: &str) -> Option<&'a Value> {
    [&state["staging"], &state["production"]]
        .into_iter()
        .find(|deployment| deployment["deployment_id"] == id)
}

#[cfg(test)]
mod http_catalog_cache_tests {
    use super::*;

    #[test]
    fn accepts_only_named_fixture_hosts_and_approved_public_literal() {
        assert!(is_fixture_authority("example.com"));
        assert!(is_fixture_authority(VIRTUAL_PUBLIC_MCP_AUTHORITY));
        assert!(!is_fixture_authority("93.184.215.15"));
        assert!(!is_fixture_authority("example.org"));
        assert!(!is_fixture_authority("93.184.215.14:80"));
        assert!(
            everruns_contracts::url_validation::validate_safe_url(
                "http://93.184.215.14/mcp/fixture/server"
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn public_literal_url_validation_skips_dns_resolution() {
        let resolve_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let resolver_calls = Arc::clone(&resolve_calls);
        let resolver = move |_: String, _: u16| {
            resolver_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Ok(Vec::new()) }
        };
        let (url, addresses) = everruns_contracts::url_validation::validate_url_with_resolver(
            "http://93.184.215.14/mcp/fixture/server",
            resolver,
        )
        .await
        .unwrap();
        assert_eq!(url.host_str(), Some(VIRTUAL_PUBLIC_MCP_AUTHORITY));
        assert!(addresses.is_empty());
        assert_eq!(resolve_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    fn dynamic_config(revision: &str) -> Value {
        json!({"scenario":"dynamic","schema_revision":revision,"attached":true})
    }

    #[test]
    fn catalog_cache_reuses_and_refreshes_for_effective_schema_config() {
        let mut state = HttpFixtureState::default();
        let revision_a = dynamic_config("A");
        let first = state.cached_catalog("fixture", "alpha", &revision_a);
        let second = state.cached_catalog("fixture", "alpha", &revision_a);
        assert!(Arc::ptr_eq(&first, &second));
        assert!(
            first[0]["inputSchema"]["properties"]["item"]["properties"]
                .get("revision_token")
                .is_none()
        );

        let revision_b = dynamic_config("B");
        let updated = state.cached_catalog("fixture", "alpha", &revision_b);
        assert!(!Arc::ptr_eq(&first, &updated));
        assert!(
            updated[0]["inputSchema"]["properties"]["item"]["properties"]
                .get("revision_token")
                .is_some()
        );
    }

    #[test]
    fn catalog_cache_is_bounded() {
        let mut state = HttpFixtureState::default();
        let config = dynamic_config("A");
        for index in 0..=HTTP_CATALOG_CACHE_ENTRIES {
            state.cached_catalog("fixture", &format!("server-{index}"), &config);
        }
        assert_eq!(state.catalog_cache.len(), HTTP_CATALOG_CACHE_ENTRIES);
    }

    #[test]
    fn control_put_invalidates_only_affected_fixture_catalogs() {
        let state = Arc::new(Mutex::new(HttpFixtureState::default()));
        let config = dynamic_config("A");
        {
            let mut locked = state.lock().unwrap();
            locked.cached_catalog("fixture", "alpha", &config);
            locked.cached_catalog("fixture", "beta", &config);
            locked.cached_catalog("other-fixture", "alpha", &config);
        }
        let response = |status, value| {
            (
                status,
                serde_json::to_vec(&value).unwrap(),
                "application/json; charset=utf-8",
            )
        };
        let server_patch = json!({"schema_revision":"B"});
        let request = HttpRequest {
            method: "PUT".into(),
            path: "/control/fixture/alpha".into(),
            headers: std::collections::HashMap::new(),
            body: serde_json::to_vec(&server_patch).unwrap(),
        };
        assert_eq!(
            route_fixture_control(&request, &["control", "fixture", "alpha"], &state, response).0,
            200
        );
        {
            let locked = state.lock().unwrap();
            assert!(
                !locked
                    .catalog_cache
                    .contains_key(&("fixture".into(), "alpha".into()))
            );
            assert!(
                locked
                    .catalog_cache
                    .contains_key(&("fixture".into(), "beta".into()))
            );
            assert!(
                locked
                    .catalog_cache
                    .contains_key(&("other-fixture".into(), "alpha".into()))
            );
        }

        let fixture_patch = json!({"schema_revision":"C"});
        let request = HttpRequest {
            method: "PUT".into(),
            path: "/control/fixture".into(),
            headers: std::collections::HashMap::new(),
            body: serde_json::to_vec(&fixture_patch).unwrap(),
        };
        assert_eq!(
            route_fixture_control(&request, &["control", "fixture"], &state, response).0,
            200
        );
        let locked = state.lock().unwrap();
        assert!(
            locked
                .catalog_cache
                .keys()
                .all(|(fixture_id, _)| fixture_id != "fixture")
        );
        assert!(
            locked
                .catalog_cache
                .contains_key(&("other-fixture".into(), "alpha".into()))
        );
    }

    #[test]
    fn list_audit_counts_full_jsonrpc_body_and_records_request_id() {
        let state = Arc::new(Mutex::new(HttpFixtureState::default()));
        state.lock().unwrap().config.insert(
            "fixture".into(),
            json!({"scenario":"dynamic","schema_revision":"A","page_size":100,"catalog_size":1}),
        );
        let request_value = json!({
            "jsonrpc":"2.0",
            "id":"request-17",
            "method":"tools/list",
            "params":{}
        });
        let request = HttpRequest {
            method: "POST".into(),
            path: "/mcp/fixture/alpha".into(),
            headers: std::collections::HashMap::from([(
                "content-type".into(),
                "application/json".into(),
            )]),
            body: serde_json::to_vec(&request_value).unwrap(),
        };
        let (status, body, _) = route_mcp_request(
            "fixture",
            "alpha",
            &request_value,
            &request,
            &state,
            |status, value| {
                (
                    status,
                    serde_json::to_vec(&value).unwrap(),
                    "application/json; charset=utf-8",
                )
            },
        );
        assert_eq!(status, 200);
        let envelope: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(envelope["id"], request_value["id"]);
        let locked = state.lock().unwrap();
        let audit = &locked.audit["fixture"][0];
        assert_eq!(audit["id"], request_value["id"]);
        assert_eq!(audit["audit_id"], 0);
        assert_eq!(audit["response_bytes"], body.len());
        assert_eq!(
            audit["result_bytes"],
            serde_json::to_vec(&envelope["result"]).unwrap().len()
        );
    }
}
