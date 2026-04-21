//! Deno Sandbox API client.
//!
//! Decision: use the public websocket sandbox API directly so the worker only
//! needs Rust dependencies at runtime.
//! Decision: open a fresh websocket per tool call; operations are short-lived
//! and this keeps session state small and deterministic.

use std::sync::Arc;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use http::Request;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, client_async_tls_with_config,
    tungstenite::{self, Message, client::IntoClientRequest},
};

use crate::{
    DENO_CONSOLE_API_BASE, DENO_DEFAULT_MEMORY_MB, DENO_RPC_TIMEOUT, DENO_SANDBOX_BASE_DOMAIN,
    DENO_STREAM_IDLE_TIMEOUT, DENO_WORKSPACE_PATH,
};

const DEFAULT_REGION: &str = "ord";

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect_async_all_addrs(
    request: Request<()>,
    connector: Connector,
) -> Result<(WsStream, http::Response<Option<Vec<u8>>>), String> {
    let uri = request.uri().clone();
    let host = uri
        .host()
        .ok_or_else(|| "Missing websocket host".to_string())?
        .to_string();
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("wss") {
            443
        } else {
            80
        });

    if let Some(proxy) = proxy_url_for_scheme(uri.scheme_str())? {
        let stream = connect_via_http_proxy(&proxy, &host, port).await?;
        return client_async_tls_with_config(request, stream, None, Some(connector))
            .await
            .map_err(map_ws_error);
    }

    let mut last_error = None;
    let addrs = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("Failed to resolve Deno sandbox host {host}:{port}: {e}"))?;

    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                match client_async_tls_with_config(
                    request.clone(),
                    stream,
                    None,
                    Some(connector.clone()),
                )
                .await
                {
                    Ok((ws, response)) => return Ok((ws, response)),
                    Err(error) => last_error = Some(map_ws_error(error)),
                }
            }
            Err(error) => {
                last_error = Some(format!(
                    "Failed to connect to Deno sandbox websocket address {addr}: {error}"
                ))
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Failed to connect to any Deno sandbox address".to_string()))
}

fn proxy_url_for_scheme(scheme: Option<&str>) -> Result<Option<reqwest::Url>, String> {
    let candidates = match scheme {
        Some("wss") | Some("https") => ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"],
        _ => ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"],
    };

    for key in candidates {
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            let url = reqwest::Url::parse(&value)
                .map_err(|e| format!("Invalid proxy URL in {key}: {e}"))?;
            return Ok(Some(url));
        }
    }

    Ok(None)
}

async fn connect_via_http_proxy(
    proxy: &reqwest::Url,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream, String> {
    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| "Proxy URL missing host".to_string())?;
    let proxy_port = proxy.port_or_known_default().unwrap_or(8080);
    let mut stream = TcpStream::connect((proxy_host, proxy_port))
        .await
        .map_err(|e| format!("Failed to connect to proxy {proxy_host}:{proxy_port}: {e}"))?;

    let mut connect_request = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n"
    );
    if !proxy.username().is_empty() || proxy.password().is_some() {
        let credentials = format!(
            "{}:{}",
            proxy.username(),
            proxy.password().unwrap_or_default()
        );
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            credentials.as_bytes(),
        );
        connect_request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    connect_request.push_str("\r\n");

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(connect_request.as_bytes())
        .await
        .map_err(|e| format!("Failed to write proxy CONNECT request: {e}"))?;

    let mut response = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let read = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("Failed to read proxy CONNECT response: {e}"))?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&buf[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 16 * 1024 {
            return Err("Proxy CONNECT response too large".to_string());
        }
    }

    let response_text = String::from_utf8_lossy(&response);
    let status_line = response_text.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") {
        return Err(format!("Proxy CONNECT failed: {status_line}"));
    }

    Ok(stream)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenoSandboxMetadata {
    pub id: String,
    pub region: String,
    pub status: String,
    #[serde(default)]
    pub labels: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct CreateSandboxRequest {
    pub region: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub memory_mb: Option<u64>,
    pub labels: serde_json::Map<String, Value>,
    pub allow_net: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreatedSandbox {
    pub sandbox_id: String,
    pub region: String,
    pub workspace_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct DenoClient {
    http: reqwest::Client,
    token: String,
    org: Option<String>,
    console_api_base: String,
    sandbox_base_domain: String,
    rpc_timeout: Duration,
    stream_idle_timeout: Duration,
    tls_connector: Connector,
}

impl DenoClient {
    pub fn new(token: String, org: Option<String>) -> Self {
        Self::with_endpoints(
            token,
            org,
            DENO_CONSOLE_API_BASE.to_string(),
            DENO_SANDBOX_BASE_DOMAIN.to_string(),
        )
    }

    pub fn with_endpoints(
        token: String,
        org: Option<String>,
        console_api_base: String,
        sandbox_base_domain: String,
    ) -> Self {
        let tls_connector = build_http11_tls_connector()
            .expect("Failed to build TLS connector for Deno sandbox WebSocket");
        Self {
            http: reqwest::Client::new(),
            token,
            org,
            console_api_base,
            sandbox_base_domain,
            rpc_timeout: DENO_RPC_TIMEOUT,
            stream_idle_timeout: DENO_STREAM_IDLE_TIMEOUT,
            tls_connector,
        }
    }

    pub fn org(&self) -> Option<&str> {
        self.org.as_deref()
    }

    pub async fn create_sandbox(
        &self,
        request: CreateSandboxRequest,
    ) -> Result<CreatedSandbox, String> {
        let region = request
            .region
            .clone()
            .unwrap_or_else(|| DEFAULT_REGION.to_string());
        let mut session = self.open_create_session(&region, &request).await?;
        session.close().await?;
        Ok(CreatedSandbox {
            sandbox_id: session.sandbox_id,
            region,
            workspace_path: DENO_WORKSPACE_PATH.to_string(),
        })
    }

    pub async fn exec(
        &self,
        sandbox_id: &str,
        region: &str,
        command: &str,
        cwd: Option<&str>,
    ) -> Result<ExecOutput, String> {
        let mut session = self.connect_sandbox(sandbox_id, region).await?;
        let output = session.exec(command, cwd).await;
        session.close().await?;
        output
    }

    pub async fn read_text_file(
        &self,
        sandbox_id: &str,
        region: &str,
        path: &str,
    ) -> Result<String, String> {
        let mut session = self.connect_sandbox(sandbox_id, region).await?;
        let result = session.read_text_file(path).await;
        session.close().await?;
        result
    }

    pub async fn write_text_file(
        &self,
        sandbox_id: &str,
        region: &str,
        path: &str,
        content: &str,
    ) -> Result<(), String> {
        let mut session = self.connect_sandbox(sandbox_id, region).await?;
        let result = session.write_text_file(path, content).await;
        session.close().await?;
        result
    }

    pub async fn delete_sandbox(&self, sandbox_id: &str, region: &str) -> Result<(), String> {
        let url = format!(
            "https://{}.{}/api/v3/sandbox/{}",
            region, self.sandbox_base_domain, sandbox_id
        );
        let mut req = self.http.delete(url).bearer_auth(&self.token);
        if let Some(org) = &self.org {
            req = req.header("X-Deno-Org", org);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Deno sandbox API: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Deno sandbox delete error ({status}): {body}"));
        }
        Ok(())
    }

    pub async fn list_sandboxes(
        &self,
        labels: &std::collections::BTreeMap<String, String>,
    ) -> Result<Vec<DenoSandboxMetadata>, String> {
        let mut url = format!("{}/api/v2/sandboxes", self.console_api_base);
        if !labels.is_empty() {
            let query = labels
                .iter()
                .map(|(key, value)| {
                    format!(
                        "labels[{}]={}",
                        urlencoding::encode(key),
                        urlencoding::encode(value)
                    )
                })
                .collect::<Vec<_>>()
                .join("&");
            url.push('?');
            url.push_str(&query);
        }

        let mut req = self.http.get(url).bearer_auth(&self.token);
        if let Some(org) = &self.org {
            req = req.header("X-Deno-Org", org);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Deno Deploy API: {e}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;
        if !status.is_success() {
            return Err(format!("Deno Deploy API error ({status}): {body}"));
        }

        let values: Vec<Value> = serde_json::from_str(&body)
            .map_err(|e| format!("Invalid JSON from Deno Deploy API: {e}"))?;
        values
            .into_iter()
            .map(|value| {
                let labels = value
                    .get("labels")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                Ok(DenoSandboxMetadata {
                    id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "Missing sandbox id".to_string())?
                        .to_string(),
                    region: value
                        .get("region")
                        .and_then(Value::as_str)
                        .unwrap_or(DEFAULT_REGION)
                        .to_string(),
                    status: value
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    labels,
                })
            })
            .collect()
    }

    async fn open_create_session(
        &self,
        region: &str,
        request: &CreateSandboxRequest,
    ) -> Result<DenoSandboxSession, String> {
        let mut config = serde_json::Map::new();
        config.insert(
            "memory_mb".to_string(),
            json!(request.memory_mb.unwrap_or(DENO_DEFAULT_MEMORY_MB)),
        );
        if let Some(timeout_seconds) = request.timeout_seconds {
            let timeout_ms = i64::try_from(timeout_seconds)
                .ok()
                .and_then(|s| s.checked_mul(1000))
                .ok_or_else(|| "Requested timeout is too large".to_string())?;
            let stop_at_ms = chrono::Utc::now()
                .timestamp_millis()
                .checked_add(timeout_ms)
                .ok_or_else(|| "Requested timeout is too large".to_string())?;
            config.insert("stop_at_ms".to_string(), json!(stop_at_ms));
        }
        if !request.labels.is_empty() {
            config.insert("labels".to_string(), Value::Object(request.labels.clone()));
        }
        if !request.allow_net.is_empty() {
            config.insert("allow_net".to_string(), json!(request.allow_net));
        }

        let url = format!(
            "wss://{}.{}/api/v3/sandboxes/create",
            region, self.sandbox_base_domain
        );
        self.connect_websocket(url, Some(Value::Object(config)))
            .await
    }

    async fn connect_sandbox(
        &self,
        sandbox_id: &str,
        region: &str,
    ) -> Result<DenoSandboxSession, String> {
        let url = format!(
            "wss://{}.{}/api/v3/sandbox/{}/connect",
            region, self.sandbox_base_domain, sandbox_id
        );
        self.connect_websocket(url, None).await
    }

    async fn connect_websocket(
        &self,
        url: String,
        config: Option<Value>,
    ) -> Result<DenoSandboxSession, String> {
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| format!("Invalid websocket request: {e}"))?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", self.token)
                .parse()
                .map_err(|e| format!("Invalid Authorization header: {e}"))?,
        );
        if let Some(org) = &self.org {
            request.headers_mut().insert(
                "X-Deno-Org",
                org.parse()
                    .map_err(|e| format!("Invalid X-Deno-Org header: {e}"))?,
            );
        }
        if let Some(config) = config {
            let encoded = base64::engine::general_purpose::STANDARD.encode(config.to_string());
            request.headers_mut().insert(
                "x-deno-sandbox-config",
                encoded
                    .parse()
                    .map_err(|e| format!("Invalid x-deno-sandbox-config header: {e}"))?,
            );
        }
        let (ws, response) = connect_async_all_addrs(request, self.tls_connector.clone()).await?;
        let sandbox_id = response
            .headers()
            .get("x-deno-sandbox-id")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| "Deno sandbox connect response missing x-deno-sandbox-id".to_string())?
            .to_string();
        Ok(DenoSandboxSession::new(
            ws,
            sandbox_id,
            self.rpc_timeout,
            self.stream_idle_timeout,
        ))
    }
}

struct DenoSandboxSession {
    ws: WsStream,
    sandbox_id: String,
    next_request_id: u64,
    rpc_timeout: Duration,
    stream_idle_timeout: Duration,
}

impl DenoSandboxSession {
    fn new(
        ws: WsStream,
        sandbox_id: String,
        rpc_timeout: Duration,
        stream_idle_timeout: Duration,
    ) -> Self {
        Self {
            ws,
            sandbox_id,
            next_request_id: 1,
            rpc_timeout,
            stream_idle_timeout,
        }
    }

    async fn close(&mut self) -> Result<(), String> {
        self.ws
            .close(None)
            .await
            .map_err(|e| format!("Failed to close Deno sandbox websocket: {e}"))
    }

    async fn exec(&mut self, command: &str, cwd: Option<&str>) -> Result<ExecOutput, String> {
        let mut spawn = serde_json::Map::new();
        spawn.insert("command".to_string(), json!("bash"));
        spawn.insert("args".to_string(), json!(["-lc", command]));
        spawn.insert("stdout".to_string(), json!("piped"));
        spawn.insert("stderr".to_string(), json!("piped"));
        if let Some(cwd) = cwd {
            spawn.insert("cwd".to_string(), json!(cwd));
        }

        let result = self.call("spawn", Value::Object(spawn), None).await?;
        let ok = extract_ok(result)?;
        let pid = ok
            .get("pid")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Deno sandbox spawn response missing pid".to_string())?;
        let stdout_id = ok.get("stdoutStreamId").and_then(Value::as_u64);
        let stderr_id = ok.get("stderrStreamId").and_then(Value::as_u64);

        let mut collectors = StreamCollectors::new(stdout_id, stderr_id);
        let wait_result = self
            .call("processWait", json!({ "pid": pid }), Some(&mut collectors))
            .await?;
        self.drain_streams(&mut collectors).await?;

        let wait_ok = extract_ok(wait_result)?;
        let exit_code = wait_ok
            .get("code")
            .and_then(Value::as_i64)
            .ok_or_else(|| "Deno sandbox processWait response missing code".to_string())?
            as i32;

        Ok(ExecOutput {
            exit_code,
            stdout: collectors.stdout_string(),
            stderr: collectors.stderr_string(),
        })
    }

    async fn read_text_file(&mut self, path: &str) -> Result<String, String> {
        let result = self
            .call("readFile", json!({ "path": path, "abortId": null }), None)
            .await?;
        let ok = extract_ok(result)?;
        let encoded = ok
            .as_str()
            .ok_or_else(|| "Deno sandbox readFile response must be a base64 string".to_string())?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("Failed to decode Deno sandbox file bytes: {e}"))?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    async fn write_text_file(&mut self, path: &str, content: &str) -> Result<(), String> {
        let result = self
            .call(
                "writeTextFile",
                json!({
                    "path": path,
                    "abortId": null,
                    "options": null,
                    "content": content,
                }),
                None,
            )
            .await?;
        let _ = extract_ok(result)?;
        Ok(())
    }

    async fn call(
        &mut self,
        method: &str,
        params: Value,
        mut collectors: Option<&mut StreamCollectors>,
    ) -> Result<Value, String> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        self.ws
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|e| format!("Failed to send Deno sandbox RPC '{method}': {e}"))?;

        loop {
            let next = timeout(self.rpc_timeout, self.ws.next())
                .await
                .map_err(|_| format!("Timed out waiting for Deno sandbox RPC '{method}'"))?;
            let Some(message) = next else {
                return Err("Deno sandbox websocket closed unexpectedly".to_string());
            };
            let message = message.map_err(|e| format!("Deno sandbox websocket error: {e}"))?;

            if let Some(value) = decode_message(message)? {
                if let Some(notification_method) = value.get("method").and_then(Value::as_str) {
                    if let Some(active) = collectors.as_deref_mut() {
                        active.handle_notification(notification_method, value.get("params"));
                    }
                    continue;
                }

                let Some(id) = value.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                if id != request_id {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    return Err(format!(
                        "Deno sandbox JSON-RPC error: {}",
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                    ));
                }
                return value
                    .get("result")
                    .cloned()
                    .ok_or_else(|| format!("Deno sandbox RPC '{method}' missing result"));
            }
        }
    }

    async fn drain_streams(&mut self, collectors: &mut StreamCollectors) -> Result<(), String> {
        while !collectors.is_complete() {
            let next = match timeout(self.stream_idle_timeout, self.ws.next()).await {
                Ok(message) => message,
                Err(_) => {
                    return Err(format!(
                        "Timed out waiting for Deno sandbox streams to finish: {}",
                        collectors.pending_streams().join(", ")
                    ));
                }
            };
            let Some(message) = next else {
                return Err(format!(
                    "Deno sandbox websocket closed before streams completed: {}",
                    collectors.pending_streams().join(", ")
                ));
            };
            let message = message.map_err(|e| format!("Deno sandbox websocket error: {e}"))?;
            if let Some(value) = decode_message(message)?
                && let Some(notification_method) = value.get("method").and_then(Value::as_str)
            {
                collectors.handle_notification(notification_method, value.get("params"));
            }
        }
        Ok(())
    }
}

fn extract_ok(result: Value) -> Result<Value, String> {
    if let Some(ok) = result.get("ok") {
        return Ok(ok.clone());
    }
    if let Some(error) = result.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown Deno sandbox error");
        return Err(message.to_string());
    }
    Err("Malformed Deno sandbox result".to_string())
}

fn decode_message(message: Message) -> Result<Option<Value>, String> {
    match message {
        Message::Text(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("Invalid JSON from Deno sandbox websocket: {e}")),
        Message::Binary(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("Invalid binary JSON from Deno sandbox websocket: {e}")),
        Message::Ping(_) | Message::Pong(_) => Ok(None),
        Message::Close(_) => Ok(None),
        Message::Frame(_) => Ok(None),
    }
}

/// Build a rustls TLS connector that advertises only `http/1.1` ALPN.
///
/// WebSocket upgrade (RFC 6455) requires HTTP/1.1. If the TLS handshake
/// negotiates h2, proxies silently convert the upgrade into a plain HTTP/2
/// GET, which returns 400 Bad Request ("invalid upgrade request").
fn build_http11_tls_connector() -> Result<Connector, String> {
    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = root_store.add(cert);
    }
    let provider = rustls::crypto::ring::default_provider();
    let mut config = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("Failed to build TLS config: {e}"))?
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Connector::Rustls(Arc::new(config)))
}

fn map_ws_error(error: tungstenite::Error) -> String {
    match error {
        tungstenite::Error::Http(response) => {
            let status = response.status();
            let body = response
                .body()
                .as_ref()
                .map(|b| {
                    let s = String::from_utf8_lossy(b);
                    // Sanitize: collapse whitespace and truncate to keep errors readable
                    let sanitized: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
                    if sanitized.len() > 256 {
                        format!("{}…", &sanitized[..256])
                    } else {
                        sanitized
                    }
                })
                .unwrap_or_default();
            if body.is_empty() {
                format!("Deno sandbox websocket HTTP error: {status}")
            } else {
                format!("Deno sandbox websocket HTTP error: {status} — {body}")
            }
        }
        other => format!("Failed to connect to Deno sandbox websocket: {other}"),
    }
}

#[derive(Default)]
struct StreamCollectors {
    stdout: Option<StreamCollector>,
    stderr: Option<StreamCollector>,
}

impl StreamCollectors {
    fn new(stdout_id: Option<u64>, stderr_id: Option<u64>) -> Self {
        Self {
            stdout: stdout_id.map(StreamCollector::new),
            stderr: stderr_id.map(StreamCollector::new),
        }
    }

    fn handle_notification(&mut self, method: &str, params: Option<&Value>) {
        let Some(params) = params else {
            return;
        };
        let stream_id = params.get("streamId").and_then(Value::as_u64);
        let Some(stream_id) = stream_id else {
            return;
        };
        let Some(target) = self.stream_mut(stream_id) else {
            return;
        };

        match method {
            "$sandbox.stream.start" => target.started = true,
            "$sandbox.stream.enqueue" => {
                if let Some(encoded) = params.get("data").and_then(Value::as_str)
                    && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded)
                {
                    target.buffer.extend_from_slice(&bytes);
                }
            }
            "$sandbox.stream.end" => target.ended = true,
            "$sandbox.stream.error" => {
                target.ended = true;
                target.error = params
                    .get("error")
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.stdout
            .as_ref()
            .is_none_or(StreamCollector::is_complete)
            && self
                .stderr
                .as_ref()
                .is_none_or(StreamCollector::is_complete)
    }

    fn stdout_string(&self) -> String {
        self.stdout
            .as_ref()
            .map(StreamCollector::as_string)
            .unwrap_or_default()
    }

    fn stderr_string(&self) -> String {
        self.stderr
            .as_ref()
            .map(StreamCollector::as_string)
            .unwrap_or_default()
    }

    fn stream_mut(&mut self, stream_id: u64) -> Option<&mut StreamCollector> {
        if self
            .stdout
            .as_ref()
            .is_some_and(|stream| stream.id == stream_id)
        {
            return self.stdout.as_mut();
        }
        if self
            .stderr
            .as_ref()
            .is_some_and(|stream| stream.id == stream_id)
        {
            return self.stderr.as_mut();
        }
        None
    }

    fn pending_streams(&self) -> Vec<&'static str> {
        let mut pending = Vec::new();
        if self
            .stdout
            .as_ref()
            .is_some_and(|stream| !stream.is_complete())
        {
            pending.push("stdout");
        }
        if self
            .stderr
            .as_ref()
            .is_some_and(|stream| !stream.is_complete())
        {
            pending.push("stderr");
        }
        pending
    }
}

struct StreamCollector {
    id: u64,
    started: bool,
    ended: bool,
    buffer: Vec<u8>,
    error: Option<String>,
}

impl StreamCollector {
    fn new(id: u64) -> Self {
        Self {
            id,
            started: false,
            ended: false,
            buffer: Vec::new(),
            error: None,
        }
    }

    fn is_complete(&self) -> bool {
        self.ended
    }

    fn as_string(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.buffer).to_string();
        if let Some(error) = &self.error {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(error);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_ok_handles_error_wrapper() {
        let err = extract_ok(json!({"error": {"message": "boom"}})).unwrap_err();
        assert_eq!(err, "boom");
    }

    #[test]
    fn stream_collectors_decode_base64_notifications() {
        let mut collectors = StreamCollectors::new(Some(7), None);
        collectors.handle_notification("$sandbox.stream.start", Some(&json!({"streamId": 7})));
        collectors.handle_notification(
            "$sandbox.stream.enqueue",
            Some(&json!({"streamId": 7, "data": "aGVsbG8="})),
        );
        collectors.handle_notification("$sandbox.stream.end", Some(&json!({"streamId": 7})));

        assert!(collectors.is_complete());
        assert_eq!(collectors.stdout_string(), "hello");
    }

    #[test]
    fn stream_collectors_report_pending_stream_names() {
        let mut collectors = StreamCollectors::new(Some(7), Some(8));
        collectors.handle_notification("$sandbox.stream.start", Some(&json!({"streamId": 7})));
        collectors.handle_notification("$sandbox.stream.end", Some(&json!({"streamId": 7})));

        assert_eq!(collectors.pending_streams(), vec!["stderr"]);
    }

    #[test]
    fn create_request_defaults_are_reasonable() {
        let request = CreateSandboxRequest {
            region: None,
            timeout_seconds: Some(1200),
            memory_mb: None,
            labels: serde_json::Map::new(),
            allow_net: vec![],
        };
        assert_eq!(request.memory_mb.unwrap_or(DENO_DEFAULT_MEMORY_MB), 1_280);
    }

    #[test]
    fn tls_connector_sets_http11_alpn() {
        let connector = build_http11_tls_connector().expect("build connector");
        let Connector::Rustls(config) = connector else {
            panic!("Expected Connector::Rustls");
        };
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    /// Verify that connect_via_http_proxy sends Proxy-Authorization when
    /// credentials are present in the proxy URL.
    #[tokio::test]
    async fn proxy_connect_sends_authorization() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.expect("read");
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            // Respond with 200 so the function returns Ok
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .expect("write");
            request
        });

        let proxy_url =
            reqwest::Url::parse(&format!("http://user:secret@127.0.0.1:{}", addr.port()))
                .expect("parse proxy URL");
        let _ = connect_via_http_proxy(&proxy_url, "example.com", 443).await;

        let request = server.await.expect("server join");
        assert!(
            request.contains("CONNECT example.com:443 HTTP/1.1"),
            "missing CONNECT line: {request}"
        );
        assert!(
            request.contains("Proxy-Authorization: Basic "),
            "missing Proxy-Authorization header: {request}"
        );
        // "user:secret" in base64
        let expected =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"user:secret");
        assert!(request.contains(&expected), "wrong credentials: {request}");
    }

    /// Verify no Proxy-Authorization header when proxy URL has no credentials.
    #[tokio::test]
    async fn proxy_connect_omits_auth_without_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.expect("read");
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .expect("write");
            request
        });

        let proxy_url = reqwest::Url::parse(&format!("http://127.0.0.1:{}", addr.port()))
            .expect("parse proxy URL");
        let _ = connect_via_http_proxy(&proxy_url, "example.com", 443).await;

        let request = server.await.expect("server join");
        assert!(
            request.contains("CONNECT example.com:443 HTTP/1.1"),
            "missing CONNECT line: {request}"
        );
        assert!(
            !request.contains("Proxy-Authorization"),
            "should not contain Proxy-Authorization: {request}"
        );
    }
}
