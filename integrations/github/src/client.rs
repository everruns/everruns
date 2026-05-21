//! Minimal GitHub REST client for blueprint-private tools.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSearchResponse {
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub incomplete_results: bool,
    #[serde(default)]
    pub items: Vec<CodeSearchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSearchItem {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub html_url: String,
    pub repository: SearchRepository,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSearchResponse {
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub incomplete_results: bool,
    #[serde(default)]
    pub items: Vec<IssueSearchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSearchItem {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub user: Option<SearchUser>,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRepository {
    pub full_name: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchUser {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubFile {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

impl GitHubFile {
    pub fn decoded_content(&self) -> Result<String, String> {
        let content = self
            .content
            .as_ref()
            .ok_or_else(|| "GitHub response did not include file content".to_string())?;
        let encoding = self.encoding.as_deref().unwrap_or("base64");
        if encoding != "base64" {
            return Err(format!("Unsupported GitHub file encoding: {encoding}"));
        }

        let normalized = content.lines().collect::<String>();
        let bytes = BASE64
            .decode(normalized)
            .map_err(|e| format!("Failed to decode GitHub file content: {e}"))?;
        String::from_utf8(bytes).map_err(|e| format!("GitHub file is not valid UTF-8: {e}"))
    }
}

pub struct GitHubClient {
    http: reqwest::Client,
    token: String,
    api_base: String,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self::with_base_url(token, crate::GITHUB_API_BASE.to_string())
    }

    pub fn with_base_url(token: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            token,
            api_base,
        }
    }

    pub async fn search_code(
        &self,
        query: &str,
        per_page: u32,
    ) -> Result<CodeSearchResponse, String> {
        let url = format!(
            "{}/search/code?q={}&per_page={}",
            self.api_base,
            encode_query(query),
            per_page
        );
        self.get_json(&url).await
    }

    pub async fn read_file(
        &self,
        repo: &str,
        path: &str,
        reference: Option<&str>,
    ) -> Result<GitHubFile, String> {
        let mut url = format!(
            "{}/repos/{}/contents/{}",
            self.api_base,
            repo,
            encode_path(path)
        );
        if let Some(reference) = reference.filter(|r| !r.trim().is_empty()) {
            url.push_str("?ref=");
            url.push_str(&encode_query(reference));
        }
        self.get_json(&url).await
    }

    pub async fn search_issues(
        &self,
        query: &str,
        per_page: u32,
    ) -> Result<IssueSearchResponse, String> {
        let url = format!(
            "{}/search/issues?q={}&per_page={}",
            self.api_base,
            encode_query(query),
            per_page
        );
        self.get_json(&url).await
    }

    async fn get_json<T>(&self, url: &str) -> Result<T, String>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "everruns-github-scout")
            .send()
            .await
            .map_err(|e| format!("Failed to connect to GitHub API: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read GitHub API response: {e}"))?;

        if !status.is_success() {
            return Err(format!("GitHub API error ({status}): {body}"));
        }

        serde_json::from_str(&body).map_err(|e| format!("Invalid JSON from GitHub API: {e}"))
    }
}

pub(crate) fn encode_query(input: &str) -> String {
    percent_encode(input, false)
}

fn encode_path(input: &str) -> String {
    input
        .split('/')
        .map(|segment| percent_encode(segment, false))
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode(input: &str, keep_slash: bool) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b'/' if keep_slash => result.push('/'),
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn encodes_query_qualifiers() {
        assert_eq!(
            encode_query("auth repo:fastify/fastify path:lib"),
            "auth%20repo%3Afastify%2Ffastify%20path%3Alib"
        );
    }

    #[test]
    fn decodes_github_file_content() {
        let file = GitHubFile {
            name: "mod.rs".into(),
            path: "src/mod.rs".into(),
            sha: "abc".into(),
            html_url: None,
            download_url: None,
            encoding: Some("base64".into()),
            content: Some("Zm4gbWFpbigpIHt9\n".into()),
        };
        assert_eq!(file.decoded_content().unwrap(), "fn main() {}");
    }

    #[tokio::test]
    async fn search_code_success() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 1,
                "incomplete_results": false,
                "items": [{
                    "name": "auth.rs",
                    "path": "src/auth.rs",
                    "sha": "abc",
                    "html_url": "https://github.com/acme/app/blob/main/src/auth.rs",
                    "repository": {
                        "full_name": "acme/app",
                        "html_url": "https://github.com/acme/app"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let client = GitHubClient::with_base_url("token".into(), mock_server.uri());
        let result = client.search_code("auth repo:acme/app", 10).await.unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].path, "src/auth.rs");
    }
}
