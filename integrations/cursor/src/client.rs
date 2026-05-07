//! Cursor Cloud Agents REST API client.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentStatus {
    Creating,
    Running,
    Finished,
    Error,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSource {
    pub repository: Option<String>,
    #[serde(rename = "ref")]
    pub ref_: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTarget {
    pub url: String,
    pub branch_name: Option<String>,
    pub pr_url: Option<String>,
    pub auto_create_pr: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorAgent {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub source: AgentSource,
    pub target: AgentTarget,
    pub created_at: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAgentsResponse {
    pub agents: Vec<CursorAgent>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListModelsResponse {
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryInfo {
    pub owner: String,
    pub name: String,
    pub repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRepositoriesResponse {
    pub repositories: Vec<RepositoryInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyInfo {
    pub api_key_name: String,
    pub created_at: String,
    pub user_email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationResponse {
    pub id: String,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub text: String,
}

pub struct LaunchAgentRequest {
    pub prompt: String,
    pub repository: String,
    pub ref_: Option<String>,
    pub model: Option<String>,
    pub auto_create_pr: Option<bool>,
    pub branch_name: Option<String>,
}

pub struct CursorClient {
    http: reqwest::Client,
    api_key: String,
    api_base: String,
}

impl CursorClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, crate::CURSOR_API_BASE.to_string())
    }

    pub fn with_base_url(api_key: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            api_base,
        }
    }

    async fn request_json<T>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, String>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = format!("{}{}", self.api_base, path);
        let mut req = self
            .http
            .request(method, &url)
            .bearer_auth(&self.api_key)
            .header("Accept", "application/json");

        if let Some(body) = body {
            req = req.header("Content-Type", "application/json").json(&body);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Cursor API: {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read Cursor API response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Cursor API error ({status}): {text}"));
        }

        if text.trim().is_empty() {
            return serde_json::from_value(Value::Object(serde_json::Map::new()))
                .map_err(|e| format!("Empty Cursor API response could not be decoded: {e}"));
        }

        serde_json::from_str(&text).map_err(|e| format!("Invalid JSON from Cursor API: {e}"))
    }

    pub async fn api_key_info(&self) -> Result<ApiKeyInfo, String> {
        self.request_json(Method::GET, "/v0/me", None).await
    }

    pub async fn list_models(&self) -> Result<ListModelsResponse, String> {
        self.request_json(Method::GET, "/v0/models", None).await
    }

    pub async fn list_repositories(&self) -> Result<ListRepositoriesResponse, String> {
        self.request_json(Method::GET, "/v0/repositories", None)
            .await
    }

    pub async fn list_agents(
        &self,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<ListAgentsResponse, String> {
        let mut path = "/v0/agents".to_string();
        let mut params = Vec::new();
        if let Some(limit) = limit {
            params.push(format!("limit={limit}"));
        }
        if let Some(cursor) = cursor {
            params.push(format!("cursor={}", urlencoding::encode(cursor)));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        self.request_json(Method::GET, &path, None).await
    }

    pub async fn get_agent(&self, id: &str) -> Result<CursorAgent, String> {
        self.request_json(Method::GET, &format!("/v0/agents/{id}"), None)
            .await
    }

    pub async fn launch_agent(&self, request: LaunchAgentRequest) -> Result<CursorAgent, String> {
        let mut body = json!({
            "prompt": { "text": request.prompt },
            "source": { "repository": request.repository },
            "target": {},
        });

        if let Some(ref_) = request.ref_ {
            body["source"]["ref"] = json!(ref_);
        }
        if let Some(model) = request.model {
            body["model"] = json!(model);
        }
        if let Some(auto_create_pr) = request.auto_create_pr {
            body["target"]["autoCreatePr"] = json!(auto_create_pr);
        }
        if let Some(branch_name) = request.branch_name {
            body["target"]["branchName"] = json!(branch_name);
        }

        self.request_json(Method::POST, "/v0/agents", Some(body))
            .await
    }

    pub async fn add_followup(&self, id: &str, prompt: &str) -> Result<Value, String> {
        self.request_json(
            Method::POST,
            &format!("/v0/agents/{id}/followup"),
            Some(json!({ "prompt": { "text": prompt } })),
        )
        .await
    }

    pub async fn get_conversation(&self, id: &str) -> Result<ConversationResponse, String> {
        self.request_json(Method::GET, &format!("/v0/agents/{id}/conversation"), None)
            .await
    }

    pub async fn delete_agent(&self, id: &str) -> Result<Value, String> {
        self.request_json(Method::DELETE, &format!("/v0/agents/{id}"), None)
            .await
    }
}

mod urlencoding {
    pub fn encode(input: &str) -> String {
        let mut result = String::with_capacity(input.len() * 2);
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push('%');
                    result.push_str(&format!("{byte:02X}"));
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn api_key_info_success() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "apiKeyName": "Test Key",
                "createdAt": "2026-01-01T00:00:00Z",
                "userEmail": "dev@example.com"
            })))
            .mount(&mock_server)
            .await;

        let client = CursorClient::with_base_url("test".into(), mock_server.uri());
        let info = client.api_key_info().await.unwrap();
        assert_eq!(info.api_key_name, "Test Key");
        assert_eq!(info.user_email.as_deref(), Some("dev@example.com"));
    }

    #[tokio::test]
    async fn launch_agent_success() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v0/agents"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "bc_abc123",
                "name": "Fix bug",
                "status": "CREATING",
                "source": { "repository": "https://github.com/acme/app", "ref": "main" },
                "target": {
                    "url": "https://cursor.com/agents?id=bc_abc123",
                    "branchName": "cursor/fix-bug",
                    "autoCreatePr": false
                },
                "createdAt": "2026-01-01T00:00:00Z"
            })))
            .mount(&mock_server)
            .await;

        let client = CursorClient::with_base_url("test".into(), mock_server.uri());
        let agent = client
            .launch_agent(LaunchAgentRequest {
                prompt: "Fix the bug".into(),
                repository: "https://github.com/acme/app".into(),
                ref_: Some("main".into()),
                model: None,
                auto_create_pr: Some(false),
                branch_name: Some("cursor/fix-bug".into()),
            })
            .await
            .unwrap();
        assert_eq!(agent.id, "bc_abc123");
        assert_eq!(agent.status, AgentStatus::Creating);
    }

    #[tokio::test]
    async fn api_error_is_returned() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v0/agents/bc_missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "code": "not_found",
                "message": "Agent not found"
            })))
            .mount(&mock_server)
            .await;

        let client = CursorClient::with_base_url("test".into(), mock_server.uri());
        let err = client.get_agent("bc_missing").await.unwrap_err();
        assert!(err.contains("404"));
        assert!(err.contains("not_found"));
    }

    #[tokio::test]
    async fn delete_agent_accepts_empty_success_body() {
        let mock_server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/v0/agents/bc_done"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&mock_server)
            .await;

        let client = CursorClient::with_base_url("test".into(), mock_server.uri());
        let deleted = client.delete_agent("bc_done").await.unwrap();
        assert_eq!(deleted, json!({}));
    }
}
