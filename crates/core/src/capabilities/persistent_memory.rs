//! Persistent Memory Capability
//!
//! Cross-session persistent memory that agents can write to and read from.
//! Memories are scoped to org-level memory stores, selected via capability config.
//! See specs/memory.md for design rationale.

use super::{Capability, CapabilityStatus};
use crate::memory_store::{MemoryContentPart, MemoryKind, MemoryLimits, MemoryQuery};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use crate::typed_id::MemoryStoreId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MEMORY_CAPABILITY_ID: &str = "memory";

// ============================================================================
// Capability
// ============================================================================

pub struct MemoryCapability;

impl Capability for MemoryCapability {
    fn id(&self) -> &str {
        MEMORY_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Persistent Memory"
    }

    fn description(&self) -> &str {
        r#"Cross-session persistent memory. Agents can save and recall facts, preferences, corrections, and procedures that survive beyond individual sessions.

> [!TIP]
> Use this for agents that learn from interactions and remember user preferences, project context, or accumulated knowledge across sessions."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("brain")
    }

    fn category(&self) -> Option<&str> {
        Some("Knowledge")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(MEMORY_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(RememberTool),
            Box::new(RecallTool),
            Box::new(ForgetTool),
        ]
    }
}

const MEMORY_SYSTEM_PROMPT: &str = r#"## Persistent Memory

You have persistent memory across sessions via `remember`, `recall`, and `forget` tools.
Use `remember` to save important facts, user preferences, corrections, or procedures.
Use `recall` to search your memory before answering questions where prior context may help.
Use `forget` to remove outdated or incorrect memories."#;

// ============================================================================
// Capability Config
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Which memory store to use (None = org default).
    #[serde(default)]
    pub store: Option<String>,
    /// Number of memories to auto-inject per turn (0 = disable passive recall).
    #[serde(default = "default_passive_recall_count")]
    pub passive_recall_count: usize,
}

fn default_passive_recall_count() -> usize {
    5
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            store: None,
            passive_recall_count: default_passive_recall_count(),
        }
    }
}

impl MemoryConfig {
    pub fn from_json(value: &Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

// ============================================================================
// Helper: resolve store ID from config or default
// ============================================================================

async fn resolve_store_id(
    config: &MemoryConfig,
    context: &ToolContext,
) -> Result<MemoryStoreId, ToolExecutionResult> {
    let backend = context
        .memory_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Memory store not available"))?;

    if let Some(ref store_str) = config.store {
        store_str
            .parse::<MemoryStoreId>()
            .map_err(|_| ToolExecutionResult::tool_error(format!("Invalid store ID: {store_str}")))
    } else {
        let org_id = context
            .org_id
            .ok_or_else(|| ToolExecutionResult::tool_error("Org ID not available for memory"))?;
        let store = backend
            .get_or_create_default_store(org_id)
            .await
            .map_err(|e| {
                ToolExecutionResult::tool_error(format!("Failed to get default store: {e}"))
            })?;
        Ok(store.id)
    }
}

// ============================================================================
// Remember Tool
// ============================================================================

pub struct RememberTool;

#[derive(Debug, Deserialize)]
struct RememberParams {
    content: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default = "default_importance")]
    importance: u8,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    images: Vec<RememberImage>,
}

#[derive(Debug, Deserialize)]
struct RememberImage {
    base64: String,
    media_type: String,
}

fn default_importance() -> u8 {
    5
}

#[async_trait]
impl Tool for RememberTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Remember")
    }

    fn description(&self) -> &str {
        "Save a fact, preference, correction, or procedure to persistent memory. Memories survive across sessions."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "maxLength": 2000,
                    "description": "1-3 sentence knowledge to persist"
                },
                "kind": {
                    "type": "string",
                    "enum": ["fact", "preference", "correction", "procedure", "context"],
                    "default": "fact",
                    "description": "Classification of this memory"
                },
                "importance": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "default": 5,
                    "description": "Importance score (1=low, 10=critical)"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "maxItems": 10,
                    "description": "Tags for categorization and recall"
                },
                "images": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "base64": { "type": "string", "description": "Base64-encoded image data" },
                            "media_type": { "type": "string", "description": "MIME type (e.g. image/png)" }
                        },
                        "required": ["base64", "media_type"],
                        "additionalProperties": false
                    },
                    "maxItems": 4,
                    "description": "Optional images to attach to this memory"
                }
            },
            "required": ["content"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("remember requires session context")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let params: RememberParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(format!("Invalid parameters: {e}")),
        };

        // Build content parts
        let mut content_parts = vec![MemoryContentPart::text(&params.content)];
        for img in &params.images {
            content_parts.push(MemoryContentPart::image(&img.base64, &img.media_type));
        }

        // Validate limits
        if let Err(e) = MemoryLimits::validate(&params.content, &params.tags, &content_parts) {
            return ToolExecutionResult::tool_error(e);
        }

        let kind = params
            .kind
            .as_deref()
            .and_then(|k| serde_json::from_value(json!(k)).ok())
            .unwrap_or(MemoryKind::Fact);

        // Parse config from empty (tools don't have direct config access, use defaults)
        let config = MemoryConfig::default();
        let store_id = match resolve_store_id(&config, context).await {
            Ok(id) => id,
            Err(e) => return e,
        };

        let backend = context.memory_store.as_ref().unwrap();

        // Check capacity
        match backend.count_active(store_id).await {
            Ok(count) if count >= MemoryLimits::MAX_MEMORIES_PER_STORE => {
                return ToolExecutionResult::tool_error(format!(
                    "Memory store is full ({} active memories, limit {})",
                    count,
                    MemoryLimits::MAX_MEMORIES_PER_STORE
                ));
            }
            Err(e) => {
                return ToolExecutionResult::tool_error(format!("Failed to check capacity: {e}"));
            }
            _ => {}
        }

        match backend
            .create_memory(
                store_id,
                params.content,
                content_parts,
                kind,
                params.importance,
                params.tags,
            )
            .await
        {
            Ok(memory) => ToolExecutionResult::success(json!({
                "memory_id": memory.id.to_string(),
                "created": true
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to save memory: {e}")),
        }
    }
}

// ============================================================================
// Recall Tool
// ============================================================================

pub struct RecallTool;

#[derive(Debug, Deserialize)]
struct RecallParams {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default = "default_recall_limit")]
    limit: usize,
}

fn default_recall_limit() -> usize {
    10
}

#[async_trait]
impl Tool for RecallTool {
    fn name(&self) -> &str {
        "recall"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Recall Memory")
    }

    fn description(&self) -> &str {
        "Search persistent memory by keyword, tag, or kind. Returns multicontent results including text and images."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword search across memory content"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Filter by tags (AND logic)"
                },
                "kind": {
                    "type": "string",
                    "enum": ["fact", "preference", "correction", "procedure", "context"],
                    "description": "Filter by memory kind"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 10,
                    "description": "Maximum number of memories to return"
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("recall requires session context")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let params: RecallParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(format!("Invalid parameters: {e}")),
        };

        let config = MemoryConfig::default();
        let store_id = match resolve_store_id(&config, context).await {
            Ok(id) => id,
            Err(e) => return e,
        };

        let kind: Option<MemoryKind> = params
            .kind
            .as_deref()
            .and_then(|k| serde_json::from_value(json!(k)).ok());

        let query = MemoryQuery {
            store_id: Some(store_id),
            query: params.query,
            tags: params.tags,
            kind,
            limit: params.limit.min(50),
        };

        let backend = context.memory_store.as_ref().unwrap();
        match backend.recall(query).await {
            Ok((memories, total)) => {
                let results: Vec<Value> = memories
                    .iter()
                    .map(|m| {
                        let parts: Vec<Value> = m
                            .content_parts
                            .iter()
                            .map(|p| match p {
                                MemoryContentPart::Text(t) => json!({
                                    "type": "text",
                                    "text": t.text
                                }),
                                MemoryContentPart::Image(i) => json!({
                                    "type": "image",
                                    "base64": i.base64,
                                    "media_type": i.media_type
                                }),
                            })
                            .collect();

                        json!({
                            "id": m.id.to_string(),
                            "kind": m.kind.to_string(),
                            "importance": m.importance,
                            "created_at": m.created_at.to_rfc3339(),
                            "content_parts": parts,
                            "tags": m.tags
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "memories": results,
                    "total": total
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to recall memories: {e}")),
        }
    }
}

// ============================================================================
// Forget Tool
// ============================================================================

pub struct ForgetTool;

#[derive(Debug, Deserialize)]
struct ForgetParams {
    memory_id: String,
}

#[async_trait]
impl Tool for ForgetTool {
    fn name(&self) -> &str {
        "forget"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Forget Memory")
    }

    fn description(&self) -> &str {
        "Deactivate a memory by ID (soft-delete). Use when a memory is outdated or incorrect."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "memory_id": {
                    "type": "string",
                    "description": "Memory ID to forget (e.g. mem_...)"
                }
            },
            "required": ["memory_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("forget requires session context")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let params: ForgetParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(format!("Invalid parameters: {e}")),
        };

        let memory_id = match params.memory_id.parse::<crate::typed_id::MemoryId>() {
            Ok(id) => id,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid memory ID: {}",
                    params.memory_id
                ));
            }
        };

        let backend = match context.memory_store.as_ref() {
            Some(b) => b,
            None => return ToolExecutionResult::tool_error("Memory store not available"),
        };

        match backend.forget(memory_id).await {
            Ok(true) => ToolExecutionResult::success(json!({
                "forgotten": true,
                "memory_id": params.memory_id
            })),
            Ok(false) => ToolExecutionResult::tool_error(format!(
                "Memory not found or already forgotten: {}",
                params.memory_id
            )),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to forget memory: {e}")),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryMemoryStore;
    use crate::memory_store::MemoryStoreBackend;
    use crate::typed_id::{MemoryId, OrgId, SessionId};
    use std::sync::Arc;

    fn test_context() -> (ToolContext, Arc<InMemoryMemoryStore>, OrgId) {
        let session_id = SessionId::new();
        let org_id = OrgId::new();
        let store = Arc::new(InMemoryMemoryStore::new());
        let ctx = ToolContext::new(session_id)
            .with_memory_store(store.clone() as Arc<dyn MemoryStoreBackend>)
            .with_org_id(org_id);
        (ctx, store, org_id)
    }

    #[test]
    fn test_capability_metadata() {
        let cap = MemoryCapability;
        assert_eq!(cap.id(), MEMORY_CAPABILITY_ID);
        assert_eq!(cap.name(), "Persistent Memory");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.category(), Some("Knowledge"));
        assert_eq!(cap.tools().len(), 3);
        assert!(cap.system_prompt_addition().is_some());
    }

    #[tokio::test]
    async fn test_remember_and_recall() {
        let (ctx, _, _) = test_context();

        // Remember
        let result = RememberTool
            .execute_with_context(
                json!({
                    "content": "The project uses Rust edition 2024",
                    "kind": "fact",
                    "importance": 7,
                    "tags": ["rust", "project"]
                }),
                &ctx,
            )
            .await;

        match &result {
            ToolExecutionResult::Success(v) => {
                assert!(v["created"].as_bool().unwrap());
                assert!(v["memory_id"].as_str().unwrap().starts_with("mem_"));
            }
            other => panic!("expected success, got {other:?}"),
        }

        // Recall by query
        let result = RecallTool
            .execute_with_context(json!({"query": "rust"}), &ctx)
            .await;

        match &result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["total"], 1);
                let memories = v["memories"].as_array().unwrap();
                assert_eq!(memories.len(), 1);
                let parts = memories[0]["content_parts"].as_array().unwrap();
                assert_eq!(parts[0]["type"], "text");
                assert!(parts[0]["text"].as_str().unwrap().contains("Rust edition"));
            }
            other => panic!("expected success, got {other:?}"),
        }

        // Recall by tag
        let result = RecallTool
            .execute_with_context(json!({"tags": ["project"]}), &ctx)
            .await;

        match &result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["total"], 1);
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_remember_with_images() {
        let (ctx, _, _) = test_context();

        let result = RememberTool
            .execute_with_context(
                json!({
                    "content": "Architecture diagram",
                    "images": [
                        { "base64": "iVBORw0KGgo=", "media_type": "image/png" }
                    ]
                }),
                &ctx,
            )
            .await;

        match &result {
            ToolExecutionResult::Success(v) => assert!(v["created"].as_bool().unwrap()),
            other => panic!("expected success, got {other:?}"),
        }

        // Recall should include image parts
        let result = RecallTool
            .execute_with_context(json!({"query": "architecture"}), &ctx)
            .await;

        match &result {
            ToolExecutionResult::Success(v) => {
                let parts = v["memories"][0]["content_parts"].as_array().unwrap();
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0]["type"], "text");
                assert_eq!(parts[1]["type"], "image");
                assert_eq!(parts[1]["media_type"], "image/png");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_forget() {
        let (ctx, _, _) = test_context();

        // Create a memory
        let result = RememberTool
            .execute_with_context(json!({"content": "temporary fact", "tags": ["temp"]}), &ctx)
            .await;

        let memory_id = match &result {
            ToolExecutionResult::Success(v) => v["memory_id"].as_str().unwrap().to_string(),
            other => panic!("expected success, got {other:?}"),
        };

        // Forget it
        let result = ForgetTool
            .execute_with_context(json!({"memory_id": memory_id}), &ctx)
            .await;

        match &result {
            ToolExecutionResult::Success(v) => assert!(v["forgotten"].as_bool().unwrap()),
            other => panic!("expected success, got {other:?}"),
        }

        // Recall should return nothing
        let result = RecallTool
            .execute_with_context(json!({"query": "temporary"}), &ctx)
            .await;

        match &result {
            ToolExecutionResult::Success(v) => assert_eq!(v["total"], 0),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_remember_validates_content_length() {
        let (ctx, _, _) = test_context();

        let long_content = "x".repeat(2001);
        let result = RememberTool
            .execute_with_context(json!({"content": long_content}), &ctx)
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("character limit"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_remember_validates_too_many_tags() {
        let (ctx, _, _) = test_context();

        let tags: Vec<String> = (0..11).map(|i| format!("tag{i}")).collect();
        let result = RememberTool
            .execute_with_context(json!({"content": "test", "tags": tags}), &ctx)
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Too many tags"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_recall_empty_store() {
        let (ctx, _, _) = test_context();

        let result = RecallTool.execute_with_context(json!({}), &ctx).await;

        match &result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["total"], 0);
                assert!(v["memories"].as_array().unwrap().is_empty());
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_forget_nonexistent() {
        let (ctx, _, _) = test_context();

        let fake_id = MemoryId::new().to_string();
        let result = ForgetTool
            .execute_with_context(json!({"memory_id": fake_id}), &ctx)
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("not found"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_remember_without_context_errors() {
        let result = RememberTool.execute(json!({"content": "test"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires session context"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_recall_by_kind() {
        let (ctx, _, _) = test_context();

        RememberTool
            .execute_with_context(
                json!({"content": "User prefers dark mode", "kind": "preference"}),
                &ctx,
            )
            .await;

        RememberTool
            .execute_with_context(json!({"content": "The sky is blue", "kind": "fact"}), &ctx)
            .await;

        let result = RecallTool
            .execute_with_context(json!({"kind": "preference"}), &ctx)
            .await;

        match &result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["total"], 1);
                assert_eq!(v["memories"][0]["kind"], "preference");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn test_memory_limits_validate_ok() {
        let parts = vec![MemoryContentPart::text("hello")];
        assert!(MemoryLimits::validate("short", &[], &parts).is_ok());
    }

    #[test]
    fn test_memory_limits_validate_image_count() {
        let parts: Vec<MemoryContentPart> = (0..5)
            .map(|_| MemoryContentPart::image("data", "image/png"))
            .collect();
        let result = MemoryLimits::validate("test", &[], &parts);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Too many images"));
    }

    #[test]
    fn test_memory_config_defaults() {
        let config = MemoryConfig::default();
        assert_eq!(config.passive_recall_count, 5);
        assert!(config.store.is_none());
    }

    #[test]
    fn test_memory_config_from_json() {
        let config = MemoryConfig::from_json(&json!({"passive_recall_count": 10}));
        assert_eq!(config.passive_recall_count, 10);
    }

    #[test]
    fn test_tools_require_context() {
        assert!(RememberTool.requires_context());
        assert!(RecallTool.requires_context());
        assert!(ForgetTool.requires_context());
    }
}
