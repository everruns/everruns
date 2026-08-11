//! Private tools for the GitHub Scout blueprint.

use async_trait::async_trait;
use everruns_core::ToolHints;
use everruns_core::tool_output_sanitizer::{
    READ_FILE_DEFAULT_LIMIT, build_text_read_file_result, parse_read_file_window_args,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::{debug, error};

use crate::client::GitHubClient;
use crate::{GITHUB_API_BASE, GITHUB_CONNECTION_PROVIDER, GITHUB_TOKEN_SECRET};

const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 30;

async fn get_github_token(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    if let Some(resolver) = context.connection_resolver.as_ref() {
        match resolver
            .get_connection_token(context.session_id, GITHUB_CONNECTION_PROVIDER)
            .await
        {
            Ok(Some(token)) if !token.trim().is_empty() => return Ok(token),
            Ok(_) => {}
            Err(e) => debug!("GitHub connection resolver failed: {e}"),
        }
    }

    if let Some(storage) = context.storage_store.as_ref() {
        match storage
            .get_secret(context.session_id, GITHUB_TOKEN_SECRET)
            .await
        {
            Ok(Some(token)) if !token.trim().is_empty() => return Ok(token),
            Ok(_) => {}
            Err(e) => {
                error!(
                    "Failed to read {GITHUB_TOKEN_SECRET} session secret for {}: {e}",
                    context.session_id
                );
                return Err(ToolExecutionResult::internal_error_msg(
                    "Failed to read GitHub token",
                ));
            }
        }
    }

    Err(ToolExecutionResult::connection_required(
        GITHUB_CONNECTION_PROVIDER,
    ))
}

fn github_client(token: String) -> GitHubClient {
    GitHubClient::new(token)
}

fn enforce_github_network_access(context: &ToolContext) -> Result<(), ToolExecutionResult> {
    if let Some(acl) = context.network_access.as_ref()
        && !acl.is_url_allowed(GITHUB_API_BASE)
    {
        return Err(ToolExecutionResult::tool_error(format!(
            "URL blocked by network access policy: {GITHUB_API_BASE}"
        )));
    }
    Ok(())
}

fn required_str<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    arguments
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
        })
}

fn limit(arguments: &Value) -> u32 {
    arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| (v as u32).clamp(1, MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT)
}

fn repo_scope(arguments: &Value) -> String {
    arguments
        .get("repos")
        .and_then(|v| v.as_array())
        .map(|repos| {
            repos
                .iter()
                .filter_map(|repo| repo.as_str())
                .map(str::trim)
                .filter(|repo| is_valid_owner_repo(repo))
                .map(|repo| format!("repo:{repo}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn is_valid_owner_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let Some(owner) = parts.next() else {
        return false;
    };
    let Some(name) = parts.next() else {
        return false;
    };
    parts.next().is_none() && is_valid_repo_segment(owner) && is_valid_repo_segment(name)
}

fn is_valid_repo_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn is_valid_repo_path(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && !path.starts_with('/')
        && path.split('/').all(|segment| {
            !segment.is_empty() && segment != "." && segment != ".." && !segment.contains('\\')
        })
}

fn scoped_query(query: &str, arguments: &Value) -> String {
    let scope = repo_scope(arguments);
    if scope.is_empty() {
        query.to_string()
    } else {
        format!("{query} {scope}")
    }
}

pub struct SearchGitHubCodeTool;

#[async_trait]
impl Tool for SearchGitHubCodeTool {
    fn name(&self) -> &str {
        "search_github_code"
    }

    fn description(&self) -> &str {
        "Search GitHub code. Use GitHub code search qualifiers such as repo:, path:, language:, symbol:, and filename:."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "GitHub code search query."
                },
                "repos": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional repositories to scope the query, in owner/repo format."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (1-30, default 10).",
                    "minimum": 1,
                    "maximum": 30
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "search_github_code requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let query = match required_str(&arguments, "query") {
            Ok(query) => query,
            Err(e) => return e,
        };
        if let Err(e) = enforce_github_network_access(context) {
            return e;
        }
        let token = match get_github_token(context).await {
            Ok(token) => token,
            Err(e) => return e,
        };

        let scoped_query = scoped_query(query, &arguments);
        match github_client(token)
            .search_code(&scoped_query, limit(&arguments))
            .await
        {
            Ok(response) => {
                let results: Vec<Value> = response
                    .items
                    .into_iter()
                    .map(|item| {
                        json!({
                            "repository": item.repository.full_name,
                            "path": item.path,
                            "name": item.name,
                            "sha": item.sha,
                            "url": item.html_url,
                        })
                    })
                    .collect();
                ToolExecutionResult::success(json!({
                    "query": scoped_query,
                    "total_count": response.total_count,
                    "incomplete_results": response.incomplete_results,
                    "results": results,
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct ReadGitHubFileTool;

#[async_trait]
impl Tool for ReadGitHubFileTool {
    fn name(&self) -> &str {
        "read_github_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 file from a GitHub repository by owner/repo, path, and optional ref."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo": {
                    "type": "string",
                    "description": "Repository in owner/repo format."
                },
                "path": {
                    "type": "string",
                    "description": "File path inside the repository."
                },
                "ref": {
                    "type": "string",
                    "description": "Optional branch, tag, or commit SHA."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Zero-based line offset to start reading from"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "default": READ_FILE_DEFAULT_LIMIT,
                    "description": "Maximum number of lines to return"
                }
            },
            "required": ["repo", "path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "read_github_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let repo = match required_str(&arguments, "repo") {
            Ok(repo) => repo,
            Err(e) => return e,
        };
        if !is_valid_owner_repo(repo) {
            return ToolExecutionResult::tool_error(
                "Invalid repo. Expected owner/repo with alphanumeric, dot, dash, or underscore segments.",
            );
        }
        let path = match required_str(&arguments, "path") {
            Ok(path) => path,
            Err(e) => return e,
        };
        if !is_valid_repo_path(path) {
            return ToolExecutionResult::tool_error(
                "Invalid path. Expected a relative repository file path without empty, dot, or dot-dot segments.",
            );
        }
        let reference = arguments.get("ref").and_then(|v| v.as_str()).map(str::trim);
        let (offset, limit) = match parse_read_file_window_args(&arguments) {
            Ok(window) => window,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };

        if let Err(e) = enforce_github_network_access(context) {
            return e;
        }
        let token = match get_github_token(context).await {
            Ok(token) => token,
            Err(e) => return e,
        };

        match github_client(token).read_file(repo, path, reference).await {
            Ok(file) => match file.decoded_content() {
                Ok(content) => {
                    let mut result = build_text_read_file_result(
                        "read_github_file",
                        &file.path,
                        &content,
                        "text",
                        offset,
                        limit,
                    );
                    result["repo"] = json!(repo);
                    result["name"] = json!(file.name);
                    result["sha"] = json!(file.sha);
                    result["url"] = json!(file.html_url);
                    result["download_url"] = json!(file.download_url);
                    ToolExecutionResult::success(result)
                }
                Err(e) => ToolExecutionResult::tool_error(e),
            },
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct SearchGitHubIssuesTool;

#[async_trait]
impl Tool for SearchGitHubIssuesTool {
    fn name(&self) -> &str {
        "search_github_issues"
    }

    fn description(&self) -> &str {
        "Search GitHub issues and pull requests. Use GitHub search qualifiers such as repo:, is:issue, is:pr, state:, author:, and label:."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "GitHub issues search query."
                },
                "repos": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional repositories to scope the query, in owner/repo format."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (1-30, default 10).",
                    "minimum": 1,
                    "maximum": 30
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "search_github_issues requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let query = match required_str(&arguments, "query") {
            Ok(query) => query,
            Err(e) => return e,
        };
        if let Err(e) = enforce_github_network_access(context) {
            return e;
        }
        let token = match get_github_token(context).await {
            Ok(token) => token,
            Err(e) => return e,
        };

        let scoped_query = scoped_query(query, &arguments);
        match github_client(token)
            .search_issues(&scoped_query, limit(&arguments))
            .await
        {
            Ok(response) => {
                let results: Vec<Value> = response
                    .items
                    .into_iter()
                    .map(|item| {
                        json!({
                            "number": item.number,
                            "title": item.title,
                            "state": item.state,
                            "author": item.user.map(|user| user.login),
                            "kind": if item.pull_request.is_some() { "pull_request" } else { "issue" },
                            "url": item.html_url,
                            "body": item.body.unwrap_or_default(),
                        })
                    })
                    .collect();
                ToolExecutionResult::success(json!({
                    "query": scoped_query,
                    "total_count": response.total_count,
                    "incomplete_results": response.incomplete_results,
                    "results": results,
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::error::Result;
    use everruns_core::traits::{
        KeyInfo, SecretInfo, SessionStorageStore, SessionStore, UserConnectionResolver,
    };
    use everruns_core::typed_id::SessionId;
    use everruns_core::{ExecutionSession, HarnessId, SessionExecutionState};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct MockConnectionResolver {
        tokens: HashMap<SessionId, String>,
    }

    #[async_trait]
    impl UserConnectionResolver for MockConnectionResolver {
        async fn get_connection_token(
            &self,
            session_id: SessionId,
            provider: &str,
        ) -> Result<Option<String>> {
            assert_eq!(provider, GITHUB_CONNECTION_PROVIDER);
            Ok(self.tokens.get(&session_id).cloned())
        }
    }

    struct MockStorageStore {
        secrets: Mutex<HashMap<String, String>>,
    }

    impl MockStorageStore {
        fn new() -> Self {
            Self {
                secrets: Mutex::new(HashMap::new()),
            }
        }

        async fn seed_secret(&self, session_id: SessionId, name: &str, value: &str) {
            self.secrets
                .lock()
                .await
                .insert(format!("{session_id}:{name}"), value.to_string());
        }
    }

    #[async_trait]
    impl SessionStorageStore for MockStorageStore {
        async fn set_value(&self, _session_id: SessionId, _key: &str, _value: &str) -> Result<()> {
            Ok(())
        }
        async fn get_value(&self, _session_id: SessionId, _key: &str) -> Result<Option<String>> {
            Ok(None)
        }
        async fn delete_value(&self, _session_id: SessionId, _key: &str) -> Result<bool> {
            Ok(false)
        }
        async fn list_keys(&self, _session_id: SessionId) -> Result<Vec<KeyInfo>> {
            Ok(vec![])
        }
        async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
            self.seed_secret(session_id, name, value).await;
            Ok(())
        }
        async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
            Ok(self
                .secrets
                .lock()
                .await
                .get(&format!("{session_id}:{name}"))
                .cloned())
        }
        async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
            Ok(self
                .secrets
                .lock()
                .await
                .remove(&format!("{session_id}:{name}"))
                .is_some())
        }
        async fn list_secrets(&self, _session_id: SessionId) -> Result<Vec<SecretInfo>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn token_prefers_connection_resolver() {
        let session_id = SessionId::new();
        let tokens = HashMap::from([(session_id, "connection-token".into())]);
        let context = ToolContext::new(session_id)
            .with_connection_resolver(Arc::new(MockConnectionResolver { tokens }));
        assert_eq!(
            get_github_token(&context).await.unwrap(),
            "connection-token"
        );
    }

    #[tokio::test]
    async fn token_falls_back_to_session_secret() {
        let session_id = SessionId::new();
        let storage = Arc::new(MockStorageStore::new());
        storage
            .seed_secret(session_id, GITHUB_TOKEN_SECRET, "secret-token")
            .await;
        let context = ToolContext::with_storage_store(session_id, storage);
        assert_eq!(get_github_token(&context).await.unwrap(), "secret-token");
    }

    #[tokio::test]
    async fn missing_token_returns_connection_required() {
        let context = ToolContext::new(SessionId::new());
        match get_github_token(&context).await {
            Err(ToolExecutionResult::ConnectionRequired { provider }) => {
                assert_eq!(provider, GITHUB_CONNECTION_PROVIDER)
            }
            other => panic!("expected connection required, got {other:?}"),
        }
    }

    struct MockSessionStore {
        session: ExecutionSession,
    }

    #[async_trait]
    impl SessionStore for MockSessionStore {
        async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
            if self.session.id == session_id {
                return Ok(Some(self.session.clone()));
            }
            Ok(None)
        }
    }

    fn child_session_with_parent(
        session_id: SessionId,
        parent_session_id: SessionId,
    ) -> ExecutionSession {
        ExecutionSession {
            id: session_id,
            workspace_id: everruns_core::WorkspaceId::from_uuid((session_id).uuid()),
            organization_id: "org_00000000000000000000000000000001".to_string(),
            harness_id: HarnessId::new(),
            agent_id: None,
            title: Some("child".to_string()),
            goal: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: SessionExecutionState::Idle,
            usage: None,
            parent_session_id: Some(parent_session_id),
            forked_from_session_id: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    // Regression: even when the session has a parent and the connection
    // resolver holds a token for the parent, the active session must not
    // fall back to the parent's GitHub credentials.
    #[tokio::test]
    async fn token_does_not_fall_back_to_parent_session_connection() {
        let session_id = SessionId::new();
        let parent_session_id = SessionId::new();
        let session = child_session_with_parent(session_id, parent_session_id);

        let context = ToolContext::new(session_id)
            .with_session_store(Arc::new(MockSessionStore { session }))
            .with_connection_resolver(Arc::new(MockConnectionResolver {
                tokens: HashMap::from([(parent_session_id, "parent-connection-token".into())]),
            }));

        match get_github_token(&context).await {
            Err(ToolExecutionResult::ConnectionRequired { provider }) => {
                assert_eq!(provider, GITHUB_CONNECTION_PROVIDER)
            }
            other => panic!("expected connection required, got {other:?}"),
        }
    }

    // Regression: same as above for the session storage secret path —
    // a secret seeded against the parent must not satisfy the child.
    #[tokio::test]
    async fn token_does_not_fall_back_to_parent_session_secret() {
        let session_id = SessionId::new();
        let parent_session_id = SessionId::new();
        let session = child_session_with_parent(session_id, parent_session_id);

        let storage = Arc::new(MockStorageStore::new());
        storage
            .seed_secret(parent_session_id, GITHUB_TOKEN_SECRET, "parent-secret")
            .await;

        let mut context = ToolContext::with_storage_store(session_id, storage);
        context = context.with_session_store(Arc::new(MockSessionStore { session }));

        match get_github_token(&context).await {
            Err(ToolExecutionResult::ConnectionRequired { provider }) => {
                assert_eq!(provider, GITHUB_CONNECTION_PROVIDER)
            }
            other => panic!("expected connection required, got {other:?}"),
        }
    }

    #[test]
    fn builds_scoped_query_from_repos() {
        let args = json!({"repos": ["owner/repo", "../bad", "acme/app"]});
        assert_eq!(
            scoped_query("auth middleware", &args),
            "auth middleware repo:owner/repo repo:acme/app"
        );
    }

    #[test]
    fn validates_owner_repo() {
        assert!(is_valid_owner_repo("owner/repo"));
        assert!(is_valid_owner_repo("owner.name/repo-name"));
        assert!(!is_valid_owner_repo("owner"));
        assert!(!is_valid_owner_repo("../repo"));
        assert!(!is_valid_owner_repo("owner/repo/extra"));
    }

    #[test]
    fn validates_repo_path() {
        assert!(is_valid_repo_path("src/lib.rs"));
        assert!(is_valid_repo_path("README.md"));
        assert!(!is_valid_repo_path(""));
        assert!(!is_valid_repo_path("/README.md"));
        assert!(!is_valid_repo_path("src//lib.rs"));
        assert!(!is_valid_repo_path("./README.md"));
        assert!(!is_valid_repo_path("../issues"));
        assert!(!is_valid_repo_path("src/../README.md"));
        assert!(!is_valid_repo_path("src\\lib.rs"));
    }
}
