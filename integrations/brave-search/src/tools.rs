//! Tool implementations for Brave Search operations.

use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::debug;

use crate::BRAVE_SEARCH_API_KEY_SECRET;
use crate::BRAVE_SEARCH_CONNECTION_PROVIDER;
use crate::client::BraveSearchClient;

// ----------------------------------------------------------------------------
// API Key Resolution
// ----------------------------------------------------------------------------

/// Resolve Brave Search API key from user connections, with session secret fallback.
async fn get_api_key(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    // Try lazy resolution from user connections (preferred: always fresh)
    if let Some(ref resolver) = context.connection_resolver {
        match resolver
            .get_connection_token(context.session_id, BRAVE_SEARCH_CONNECTION_PROVIDER)
            .await
        {
            Ok(Some(token)) if !token.is_empty() => return Ok(token),
            Ok(_) => {}
            Err(e) => debug!("Brave Search connection resolver failed: {e}"),
        }
    }

    // Fallback: session secret
    if let Some(ref storage) = context.storage_store {
        match storage
            .get_secret(context.session_id, BRAVE_SEARCH_API_KEY_SECRET)
            .await
        {
            Ok(Some(key)) if !key.is_empty() => return Ok(key),
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to read {BRAVE_SEARCH_API_KEY_SECRET} secret: {e}");
                return Err(ToolExecutionResult::internal_error_msg(format!(
                    "Failed to read API key: {e}"
                )));
            }
        }
    }

    Err(ToolExecutionResult::tool_error(
        "Brave Search API key not configured. Connect Brave Search in Settings > Connections, \
         or use `secret_store set BRAVE_SEARCH_API_KEY <your-key>`. \
         Get a free API key at https://brave.com/search/api/",
    ))
}

// ----------------------------------------------------------------------------
// BraveWebSearchTool
// ----------------------------------------------------------------------------

pub struct BraveWebSearchTool;

#[async_trait]
impl Tool for BraveWebSearchTool {
    fn name(&self) -> &str {
        "brave_web_search"
    }

    fn description(&self) -> &str {
        "Search the web using Brave Search. Returns relevant web results \
         including titles, URLs, and descriptions."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (1-20, default: 10)",
                    "minimum": 1,
                    "maximum": 20
                },
                "offset": {
                    "type": "integer",
                    "description": "Pagination offset (default: 0)",
                    "minimum": 0
                },
                "freshness": {
                    "type": "string",
                    "enum": ["pd", "pw", "pm", "py"],
                    "description": "Time filter: pd (past day), pw (past week), pm (past month), py (past year)"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "brave_web_search requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let query = match arguments.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q,
            _ => {
                return ToolExecutionResult::tool_error("Missing required parameter: query");
            }
        };

        let count = arguments
            .get("count")
            .and_then(|v| v.as_u64())
            .map(|c| c.min(20) as u32);
        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|o| o as u32);
        let freshness = arguments.get("freshness").and_then(|v| v.as_str());

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };

        let client = BraveSearchClient::new(api_key);

        match client.web_search(query, count, offset, freshness).await {
            Ok(response) => {
                let results = response.web.map(|w| w.results).unwrap_or_default();

                let result_items: Vec<Value> = results
                    .iter()
                    .map(|r| {
                        let mut item = json!({
                            "title": r.title,
                            "url": r.url,
                            "description": r.description,
                        });
                        if let Some(ref age) = r.age {
                            item["age"] = json!(age);
                        }
                        item
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "query": query,
                    "results": result_items,
                    "count": result_items.len()
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use everruns_core::error::Result;
    use everruns_core::traits::{KeyInfo, SecretInfo, SessionStorageStore, UserConnectionResolver};
    use everruns_core::typed_id::SessionId;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// In-memory mock of SessionStorageStore for testing.
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
            let key = format!("{}:{}", session_id, name);
            self.secrets.lock().await.insert(key, value.to_string());
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
            let key = format!("{session_id}:{name}");
            self.secrets.lock().await.insert(key, value.to_string());
            Ok(())
        }
        async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
            let key = format!("{session_id}:{name}");
            Ok(self.secrets.lock().await.get(&key).cloned())
        }
        async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
            let key = format!("{session_id}:{name}");
            Ok(self.secrets.lock().await.remove(&key).is_some())
        }
        async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
            let prefix = format!("{session_id}:");
            let secrets = self.secrets.lock().await;
            Ok(secrets
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .map(|k| SecretInfo {
                    name: k.strip_prefix(&prefix).unwrap_or(k).to_string(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
                .collect())
        }
    }

    struct MockConnectionResolver {
        token: Option<String>,
    }

    #[async_trait]
    impl UserConnectionResolver for MockConnectionResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> Result<Option<String>> {
            Ok(self.token.clone())
        }
    }

    #[tokio::test]
    async fn test_brave_web_search_without_context() {
        let tool = BraveWebSearchTool;
        let result = tool.execute(json!({"query": "test"})).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_brave_web_search_missing_query() {
        let tool = BraveWebSearchTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool.execute_with_context(json!({}), &context).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("Missing required parameter"),
                    "Expected missing param error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for missing query"),
        }
    }

    #[tokio::test]
    async fn test_brave_web_search_empty_query() {
        let tool = BraveWebSearchTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool
            .execute_with_context(json!({"query": ""}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("Missing required parameter"),
                    "Expected missing param error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for empty query"),
        }
    }

    #[tokio::test]
    async fn test_brave_web_search_no_api_key() {
        let tool = BraveWebSearchTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool
            .execute_with_context(json!({"query": "test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("API key not configured"),
                    "Expected API key error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for missing API key"),
        }
    }

    #[tokio::test]
    async fn test_brave_web_search_with_session_secret() {
        // API key set as session secret — tool should attempt search.
        // Will fail at HTTP call (no real API), but token resolution should succeed.
        let tool = BraveWebSearchTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "BRAVE_SEARCH_API_KEY", "test_key")
            .await;
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool
            .execute_with_context(json!({"query": "test"}), &context)
            .await;
        // Should fail at HTTP call, not at API key resolution
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(
                !msg.contains("API key not configured"),
                "API key should have been resolved from session secret, got: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn test_brave_web_search_prefers_connection_resolver() {
        let tool = BraveWebSearchTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "BRAVE_SEARCH_API_KEY", "fallback_key")
            .await;
        let resolver = Arc::new(MockConnectionResolver {
            token: Some("preferred_key".to_string()),
        });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let result = tool
            .execute_with_context(json!({"query": "test"}), &context)
            .await;
        // Should fail at HTTP call, not at API key resolution
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(
                !msg.contains("API key not configured"),
                "API key should have been resolved from connection resolver, got: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn test_get_api_key_prefers_connection_resolver() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "BRAVE_SEARCH_API_KEY", "fallback")
            .await;
        let resolver = Arc::new(MockConnectionResolver {
            token: Some("preferred".to_string()),
        });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let key = get_api_key(&context).await;
        assert!(key.is_ok());
        assert_eq!(key.unwrap(), "preferred");
    }

    #[tokio::test]
    async fn test_get_api_key_falls_back_to_secret() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "BRAVE_SEARCH_API_KEY", "secret_key")
            .await;
        let context = ToolContext::with_storage_store(session_id, store);
        let key = get_api_key(&context).await;
        assert!(key.is_ok());
        assert_eq!(key.unwrap(), "secret_key");
    }

    #[tokio::test]
    async fn test_get_api_key_skips_empty_connection_token() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "BRAVE_SEARCH_API_KEY", "fallback")
            .await;
        let resolver = Arc::new(MockConnectionResolver {
            token: Some("".to_string()),
        });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let key = get_api_key(&context).await;
        assert!(key.is_ok());
        assert_eq!(key.unwrap(), "fallback");
    }

    #[tokio::test]
    async fn test_get_api_key_returns_error_when_none() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let resolver = Arc::new(MockConnectionResolver { token: None });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let key = get_api_key(&context).await;
        assert!(key.is_err());
    }

    #[test]
    fn test_search_schema_has_required_query() {
        let tool = BraveWebSearchTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"query"));
        assert_eq!(required_strs.len(), 1);
    }

    #[test]
    fn test_search_schema_has_optional_fields() {
        let tool = BraveWebSearchTool;
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["count"].is_object());
        assert!(schema["properties"]["offset"].is_object());
        assert!(schema["properties"]["freshness"].is_object());
    }

    #[test]
    fn test_search_schema_freshness_enum() {
        let tool = BraveWebSearchTool;
        let schema = tool.parameters_schema();
        let freshness_enum = &schema["properties"]["freshness"]["enum"];
        let values: Vec<&str> = freshness_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(values.contains(&"pd"));
        assert!(values.contains(&"pw"));
        assert!(values.contains(&"pm"));
        assert!(values.contains(&"py"));
    }

    #[test]
    fn test_search_schema_disallows_additional_properties() {
        let tool = BraveWebSearchTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn test_tool_name() {
        let tool = BraveWebSearchTool;
        assert_eq!(tool.name(), "brave_web_search");
    }

    #[test]
    fn test_tool_has_description() {
        let tool = BraveWebSearchTool;
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("Brave Search"));
    }

    #[test]
    fn test_tool_requires_context() {
        let tool = BraveWebSearchTool;
        assert!(tool.requires_context());
    }
}
