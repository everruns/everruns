//! E2B API client.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use connectrpc::Protocol;
use connectrpc::client::{ClientConfig, HttpClient};
use rustls::RootCertStore;
use serde_json::{Value, json};

use crate::state::{E2BSandboxCreateResponse, E2BSandboxDetail, SandboxState};
use crate::{E2B_API_BASE, E2B_DEFAULT_TIMEOUT_SECS, E2B_ENVD_PORT};

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
            .map_err(|e| format!("Failed to reach E2B API: {e}"))?;
        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read E2B API response: {e}"))?;

        if !status.is_success() {
            return Err(format!("E2B API error ({status}): {body_text}"));
        }

        if body_text.is_empty() {
            return Ok(json!({}));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid E2B JSON: {e}"))
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

    pub async fn create_sandbox(
        &self,
        template_id: &str,
        timeout_seconds: u64,
        metadata: Value,
        env_vars: Value,
    ) -> Result<E2BSandboxCreateResponse, String> {
        let value = self
            .management_request(
                reqwest::Method::POST,
                "/sandboxes",
                Some(Self::create_sandbox_body(
                    template_id,
                    timeout_seconds,
                    metadata,
                    env_vars,
                )),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| format!("Failed to parse sandbox response: {e}"))
    }

    pub async fn get_sandbox(&self, sandbox_id: &str) -> Result<E2BSandboxDetail, String> {
        let value = self
            .management_request(
                reqwest::Method::GET,
                &format!("/sandboxes/{sandbox_id}"),
                None,
            )
            .await?;
        serde_json::from_value(value).map_err(|e| format!("Failed to parse sandbox detail: {e}"))
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
            .map_err(|e| format!("Failed to parse sandbox resume response: {e}"))
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
            .map_err(|e| format!("Failed to read sandbox file: {e}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read sandbox file body: {e}"))?;
        if !status.is_success() {
            return Err(format!("E2B sandbox file API error ({status}): {body}"));
        }
        Ok(body)
    }

    pub async fn write_file(
        &self,
        state: &SandboxState,
        path: &str,
        content: &str,
    ) -> Result<(), String> {
        let url = format!(
            "{}/files?path={}",
            self.envd_base_url(state),
            urlencoding::encode(path)
        );
        let part = reqwest::multipart::Part::text(content.to_string())
            .file_name(path.rsplit('/').next().unwrap_or("file").to_string());
        let form = reqwest::multipart::Form::new().part("file", part);
        let response = self
            .http
            .post(&url)
            .headers(self.envd_headers(state)?)
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Failed to write sandbox file: {e}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read sandbox write response: {e}"))?;
        if !status.is_success() {
            return Err(format!("E2B sandbox file API error ({status}): {body}"));
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
        let client = self.process_client(state)?;
        let request = proto::process::StartRequest {
            process: buffa::MessageField::some(proto::process::ProcessConfig {
                cmd: "/bin/bash".to_string(),
                args: vec!["-l".to_string(), "-c".to_string(), command.to_string()],
                envs: std::collections::HashMap::new(),
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
            .map_err(|e| format!("Failed to start sandbox command: {e}"))?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = -1;
        let mut error = None;

        while let Some(message) = stream
            .message()
            .await
            .map_err(|e| format!("Failed reading sandbox command stream: {e}"))?
        {
            let Some(event) = message.event.as_option() else {
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
                .map_err(|e| format!("Invalid sandbox id header: {e}"))?,
        );
        headers.insert(
            "E2b-Sandbox-Port",
            HeaderValue::from_str(&E2B_ENVD_PORT.to_string())
                .map_err(|e| format!("Invalid sandbox port header: {e}"))?,
        );
        if let Some(token) = state.envd_access_token.as_ref() {
            headers.insert(
                "X-Access-Token",
                HeaderValue::from_str(token)
                    .map_err(|e| format!("Invalid envd access token header: {e}"))?,
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
                    .map_err(|e| format!("Failed to load native CA roots: {e}"))?;
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
            .map_err(|e| format!("Invalid envd URI: {e}"))?;
        let mut config = ClientConfig::new(uri)
            .protocol(Protocol::Connect)
            .default_timeout(Duration::from_secs(E2B_DEFAULT_TIMEOUT_SECS));
        for (name, value) in self.envd_headers(state)?.iter() {
            config = config.default_header(name, value.clone());
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
}
