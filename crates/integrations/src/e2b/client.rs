//! E2B API client.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use connectrpc::Protocol;
use connectrpc::client::{ClientConfig, HttpClient};
use rustls::RootCertStore;
use serde_json::{Value, json};

use crate::e2b::network::E2BEgress;
use crate::e2b::state::{E2BSandboxCreateResponse, E2BSandboxDetail, SandboxState};
use crate::e2b::{E2B_API_BASE, E2B_DEFAULT_TIMEOUT_SECS, E2B_ENVD_PORT};

mod proto {
    #![allow(clippy::upper_case_acronyms)]
    include!(concat!(env!("OUT_DIR"), "/_e2b.rs"));
}

pub struct E2BClient {
    http: reqwest::Client,
    api_key: String,
    api_base: String,
    process_tls_config: OnceLock<Result<Arc<rustls::ClientConfig>, String>>,
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub error: Option<String>,
}

impl E2BClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, E2B_API_BASE.to_string())
    }

    pub fn with_base_url(api_key: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            api_base,
            process_tls_config: OnceLock::new(),
        }
    }

    async fn management_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", self.api_base, path);
        let mut req = self
            .http
            .request(method, &url)
            .header("X-API-KEY", &self.api_key)
            .header("Content-Type", "application/json");

        if let Some(body) = body {
            req = req.json(&body);
        }

        let response = req
            .send()
            .await
            .map_err(|_| "Failed to reach E2B API".to_string())?;
        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|_| "Failed to read E2B API response".to_string())?;

        // THREAT[TM-FW-001]: upstream errors must never echo service credentials.
        if !status.is_success() {
            return Err(format!("E2B API error ({status})"));
        }

        if body_text.is_empty() {
            return Ok(json!({}));
        }

        serde_json::from_str(&body_text).map_err(|_| "Invalid E2B JSON".to_string())
    }

    fn create_sandbox_body(
        template_id: &str,
        timeout_seconds: u64,
        metadata: Value,
        env_vars: Value,
    ) -> Value {
        json!({
            "templateID": template_id,
            "timeout": timeout_seconds,
            "autoPause": true,
            "allowInternetAccess": true,
            "metadata": metadata,
            "envVars": env_vars,
        })
    }

    /// Create a sandbox with open egress.
    pub async fn create_sandbox(
        &self,
        template_id: &str,
        timeout_seconds: u64,
        metadata: Value,
        env_vars: Value,
    ) -> Result<E2BSandboxCreateResponse, String> {
        self.create_sandbox_with_egress(
            template_id,
            timeout_seconds,
            metadata,
            env_vars,
            &E2BEgress::open(),
        )
        .await
    }

    /// Create a sandbox whose outbound traffic E2B restricts to `egress`.
    pub async fn create_sandbox_with_egress(
        &self,
        template_id: &str,
        timeout_seconds: u64,
        metadata: Value,
        env_vars: Value,
        egress: &E2BEgress,
    ) -> Result<E2BSandboxCreateResponse, String> {
        let mut body = Self::create_sandbox_body(template_id, timeout_seconds, metadata, env_vars);
        body["allowInternetAccess"] = json!(egress.allow_internet_access);
        if let Some(network) = &egress.network {
            body["network"] = network.clone();
        }
        let value = self
            .management_request(reqwest::Method::POST, "/sandboxes", Some(body))
            .await?;
        serde_json::from_value(value).map_err(|_| "Failed to parse sandbox response".to_string())
    }

    /// Replace a running sandbox's egress rules.
    pub async fn update_network(&self, sandbox_id: &str, network: Value) -> Result<(), String> {
        self.management_request(
            reqwest::Method::PUT,
            &format!("/sandboxes/{sandbox_id}/network"),
            Some(network),
        )
        .await?;
        Ok(())
    }

    pub async fn get_sandbox(&self, sandbox_id: &str) -> Result<E2BSandboxDetail, String> {
        let value = self
            .management_request(
                reqwest::Method::GET,
                &format!("/sandboxes/{sandbox_id}"),
                None,
            )
            .await?;
        serde_json::from_value(value).map_err(|_| "Failed to parse sandbox detail".to_string())
    }

    /// One page of `GET /v2/sandboxes`, filtered by `metadata` (every pair
    /// must match) and `states`. Returns the raw items and the `x-next-token`
    /// cursor for the next page, if any.
    pub async fn list_sandboxes(
        &self,
        metadata: &std::collections::BTreeMap<String, String>,
        states: &[&str],
        next_token: Option<&str>,
    ) -> Result<(Vec<Value>, Option<String>), String> {
        // The metadata filter is itself a URL-encoded `k=v&k=v` query, sent as
        // one (again encoded) parameter, which is what the E2B SDKs send.
        let filter = metadata
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let mut url = format!(
            "{}/v2/sandboxes?limit=100&metadata={}",
            self.api_base,
            urlencoding::encode(&filter)
        );
        if !states.is_empty() {
            url.push_str("&state=");
            url.push_str(&urlencoding::encode(&states.join(",")));
        }
        if let Some(token) = next_token {
            url.push_str("&nextToken=");
            url.push_str(&urlencoding::encode(token));
        }
        let response = self
            .http
            .get(&url)
            .header("X-API-KEY", &self.api_key)
            .send()
            .await
            .map_err(|_| "Failed to reach E2B API".to_string())?;
        let status = response.status();
        let next = response
            .headers()
            .get("x-next-token")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .map(str::to_string);
        let body = response
            .text()
            .await
            .map_err(|_| "Failed to read E2B API response".to_string())?;
        if !status.is_success() {
            return Err(format!("E2B API error ({status})"));
        }
        let items: Vec<Value> =
            serde_json::from_str(&body).map_err(|_| "Invalid E2B JSON".to_string())?;
        Ok((items, next))
    }

    pub async fn pause_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(
            reqwest::Method::POST,
            &format!("/sandboxes/{sandbox_id}/pause"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn resume_sandbox(
        &self,
        sandbox_id: &str,
        timeout_seconds: u64,
    ) -> Result<E2BSandboxCreateResponse, String> {
        let value = self
            .management_request(
                reqwest::Method::POST,
                &format!("/sandboxes/{sandbox_id}/resume"),
                Some(json!({
                    "autoPause": true,
                    "timeout": timeout_seconds,
                })),
            )
            .await?;
        serde_json::from_value(value)
            .map_err(|_| "Failed to parse sandbox resume response".to_string())
    }

    /// Copy a running sandbox: files, memory and processes. The source keeps
    /// running (its open connections drop). The copy inherits the source's
    /// metadata, egress rules and auto-pause; callers re-apply their own.
    pub async fn fork_sandbox(
        &self,
        sandbox_id: &str,
        timeout_seconds: u64,
    ) -> Result<E2BSandboxCreateResponse, String> {
        let value = self
            .management_request(
                reqwest::Method::POST,
                &format!("/sandboxes/{sandbox_id}/fork"),
                Some(json!({ "count": 1, "timeout": timeout_seconds })),
            )
            .await?;
        let first = value
            .as_array()
            .and_then(|items| items.first())
            .ok_or_else(|| "E2B fork returned no sandbox".to_string())?;
        if let Some(sandbox) = first.get("sandbox") {
            return serde_json::from_value(sandbox.clone())
                .map_err(|_| "Failed to parse sandbox fork response".to_string());
        }
        // THREAT[TM-FW-001]: report that the fork failed, not E2B's text.
        Err("E2B could not fork the sandbox".to_string())
    }

    pub async fn delete_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        self.management_request(
            reqwest::Method::DELETE,
            &format!("/sandboxes/{sandbox_id}"),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn set_timeout(&self, sandbox_id: &str, timeout_seconds: u64) -> Result<(), String> {
        self.management_request(
            reqwest::Method::POST,
            &format!("/sandboxes/{sandbox_id}/timeout"),
            Some(json!({ "timeout": timeout_seconds })),
        )
        .await?;
        Ok(())
    }

    pub async fn read_file(&self, state: &SandboxState, path: &str) -> Result<String, String> {
        let bytes = self.read_file_bytes(state, path).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Read a sandbox file as raw bytes (images, archives).
    pub async fn read_file_bytes(
        &self,
        state: &SandboxState,
        path: &str,
    ) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/files?path={}",
            self.envd_base_url(state),
            urlencoding::encode(path)
        );
        let response = self
            .http
            .get(&url)
            .headers(self.envd_headers(state)?)
            .send()
            .await
            .map_err(|_| "Failed to read sandbox file".to_string())?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|_| "Failed to read sandbox file body".to_string())?;
        if !status.is_success() {
            return Err(format!("E2B sandbox file API error ({status})"));
        }
        Ok(body.to_vec())
    }

    pub async fn write_file(
        &self,
        state: &SandboxState,
        path: &str,
        content: &str,
    ) -> Result<(), String> {
        self.write_file_bytes(state, path, content.as_bytes()).await
    }

    pub async fn write_file_bytes(
        &self,
        state: &SandboxState,
        path: &str,
        content: &[u8],
    ) -> Result<(), String> {
        let url = format!(
            "{}/files?path={}",
            self.envd_base_url(state),
            urlencoding::encode(path)
        );
        let part = reqwest::multipart::Part::bytes(content.to_vec())
            .file_name(path.rsplit('/').next().unwrap_or("file").to_string());
        let form = reqwest::multipart::Form::new().part("file", part);
        let response = self
            .http
            .post(&url)
            .headers(self.envd_headers(state)?)
            .multipart(form)
            .send()
            .await
            .map_err(|_| "Failed to write sandbox file".to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("E2B sandbox file API error ({status})"));
        }
        Ok(())
    }

    pub async fn exec(
        &self,
        state: &SandboxState,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, String> {
        let args = vec!["-l".to_string(), "-c".to_string(), command.to_string()];
        self.exec_argv(state, "/bin/bash", &args, &[], cwd, timeout_ms)
            .await
    }

    /// Run `program` with `args` as its argv, with no shell in between, so
    /// arguments reach the program byte for byte (the computer-use desktop
    /// passes model-typed text this way).
    pub async fn exec_argv(
        &self,
        state: &SandboxState,
        program: &str,
        args: &[String],
        envs: &[(&str, &str)],
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, String> {
        let client = self.process_client(state)?;
        let request = proto::process::StartRequest {
            process: buffa::MessageField::some(proto::process::ProcessConfig {
                cmd: program.to_string(),
                args: args.to_vec(),
                envs: envs
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
                cwd: cwd.map(|v| v.to_string()),
                ..Default::default()
            }),
            stdin: Some(false),
            ..Default::default()
        };

        let options = connectrpc::client::CallOptions::default()
            .with_timeout(Duration::from_millis(timeout_ms.unwrap_or(60_000)));
        let mut stream = client
            .start_with_options(request, options)
            .await
            .map_err(|_| "Failed to start sandbox command".to_string())?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = -1;
        let mut error = None;

        while let Some(message) = stream
            .message()
            .await
            .map_err(|_| "Failed reading sandbox command stream".to_string())?
        {
            let Some(event) = message.view().event.as_option() else {
                continue;
            };
            match &event.event {
                Some(proto::process::process_event::EventView::Data(data)) => match &data.output {
                    Some(proto::process::process_event::data_event::OutputView::Stdout(bytes)) => {
                        stdout.push_str(&String::from_utf8_lossy(bytes));
                    }
                    Some(proto::process::process_event::data_event::OutputView::Stderr(bytes)) => {
                        stderr.push_str(&String::from_utf8_lossy(bytes));
                    }
                    Some(proto::process::process_event::data_event::OutputView::Pty(bytes)) => {
                        stdout.push_str(&String::from_utf8_lossy(bytes));
                    }
                    None => {}
                },
                Some(proto::process::process_event::EventView::End(end)) => {
                    exit_code = end.exit_code;
                    error = end.error.map(|value| value.to_string());
                    break;
                }
                _ => {}
            }
        }

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
            error,
        })
    }

    fn envd_base_url(&self, state: &SandboxState) -> String {
        format!(
            "https://{}-{}.{}",
            E2B_ENVD_PORT, state.sandbox_id, state.sandbox_domain
        )
    }

    fn envd_headers(&self, state: &SandboxState) -> Result<reqwest::header::HeaderMap, String> {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            "E2b-Sandbox-Id",
            HeaderValue::from_str(&state.sandbox_id)
                .map_err(|_| "Invalid sandbox id header".to_string())?,
        );
        headers.insert(
            "E2b-Sandbox-Port",
            HeaderValue::from_str(&E2B_ENVD_PORT.to_string())
                .map_err(|_| "Invalid sandbox port header".to_string())?,
        );
        if let Some(token) = state.envd_access_token.as_ref() {
            headers.insert(
                "X-Access-Token",
                HeaderValue::from_str(token)
                    .map_err(|_| "Invalid envd access token header".to_string())?,
            );
        }
        Ok(headers)
    }

    fn process_client(
        &self,
        state: &SandboxState,
    ) -> Result<proto::process::ProcessClient<HttpClient>, String> {
        let tls_config = self.process_tls_config.get_or_init(|| {
            let mut roots = RootCertStore::empty();
            let certs = rustls_native_certs::load_native_certs();
            for cert in certs.certs {
                roots
                    .add(cert)
                    .map_err(|_| "Failed to load native CA roots".to_string())?;
            }

            Ok::<Arc<rustls::ClientConfig>, String>(Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            ))
        });
        let http = HttpClient::with_tls(tls_config.clone()?);
        let uri: http::Uri = self
            .envd_base_url(state)
            .parse()
            .map_err(|_| "Invalid envd URI".to_string())?;
        let mut config = ClientConfig::new(uri)
            .with_protocol(Protocol::Connect)
            .with_default_timeout(Duration::from_secs(E2B_DEFAULT_TIMEOUT_SECS));
        for (name, value) in self.envd_headers(state)?.iter() {
            config = config.with_default_header(name, value.clone());
        }
        Ok(proto::process::ProcessClient::new(http, config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_sandbox_body_matches_expected_shape() {
        let body = E2BClient::create_sandbox_body(
            "base",
            3600,
            json!({"everruns": "true"}),
            json!({"HELLO": "world"}),
        );
        assert_eq!(
            body,
            json!({
                "templateID": "base",
                "timeout": 3600,
                "autoPause": true,
                "allowInternetAccess": true,
                "metadata": {"everruns": "true"},
                "envVars": {"HELLO": "world"},
            })
        );
    }

    #[test]
    fn envd_headers_include_expected_auth_headers() {
        let client = E2BClient::new("key".to_string());
        let state = SandboxState {
            sandbox_id: "sb_test".to_string(),
            sandbox_domain: "example.e2b.app".to_string(),
            envd_version: "0.1.0".to_string(),
            envd_access_token: Some("envd_token".to_string()),
            workspace_path: "/home/user".to_string(),
            started_at: "2026-03-22T00:00:00Z".to_string(),
            timeout_seconds: 3600,
        };

        let headers = client.envd_headers(&state).unwrap();
        assert_eq!(headers.get("e2b-sandbox-id").unwrap(), "sb_test");
        assert_eq!(headers.get("e2b-sandbox-port").unwrap(), "49983");
        assert_eq!(headers.get("x-access-token").unwrap(), "envd_token");
    }

    #[test]
    fn envd_headers_omit_access_token_when_absent() {
        let client = E2BClient::new("key".to_string());
        let state = SandboxState {
            sandbox_id: "sb_test".to_string(),
            sandbox_domain: "example.e2b.app".to_string(),
            envd_version: "0.1.0".to_string(),
            envd_access_token: None,
            workspace_path: "/home/user".to_string(),
            started_at: "2026-03-22T00:00:00Z".to_string(),
            timeout_seconds: 3600,
        };

        let headers = client.envd_headers(&state).unwrap();
        assert_eq!(headers.get("e2b-sandbox-id").unwrap(), "sb_test");
        assert!(headers.get("x-access-token").is_none());
    }

    #[tokio::test]
    async fn credential_errors_omit_upstream_bodies() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let secret = "e2b-secret-sentinel";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandboxes/sandbox/pause"))
            .respond_with(ResponseTemplate::new(403).set_body_string(secret))
            .mount(&server)
            .await;
        let client = E2BClient::with_base_url(secret.into(), server.uri());
        let error = client.pause_sandbox("sandbox").await.unwrap_err();
        assert!(error.contains("403"));
        assert!(!error.contains(secret));
    }
}
