//! Session Storage Capability
//!
//! This capability provides tools for session-scoped key/value and secret storage.
//! Data persists for the session duration.
//!
//! Tools provided:
//! - `kv_store`: Key/value storage operations (set, get, delete, list)
//! - `secret_store`: Encrypted secret storage operations (set, get, delete, list)

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Session Storage capability - provides key/value and secret storage for sessions
pub struct SessionStorageCapability;

impl Capability for SessionStorageCapability {
    fn id(&self) -> &str {
        "session_storage"
    }

    fn name(&self) -> &str {
        "Session Storage"
    }

    fn description(&self) -> &str {
        r#"Tools to store and retrieve key/value pairs and encrypted secrets within a session.

> [!NOTE]
> Data persists for the session duration. Secrets are encrypted at rest.

> [!TIP]
> Use key/value storage for general data. Use secrets for sensitive information like API keys or tokens."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("database")
    }

    fn category(&self) -> Option<&str> {
        Some("Storage")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to session storage tools for persisting data within the session.

`kv_store` - Key/value storage (plain text):
- operation: "set" - Store a key/value pair (key, value required)
- operation: "get" - Retrieve a value (key required)
- operation: "delete" - Delete a key/value pair (key required)
- operation: "list" - List all stored keys

`secret_store` - Secret storage (encrypted at rest):
- operation: "set" - Store a secret (name, value required)
- operation: "get" - Retrieve a secret (name required)
- operation: "delete" - Delete a secret (name required)
- operation: "list" - List all secret names

Best practices:
- Use kv_store for general data like state, preferences, or intermediate results
- Use secret_store for sensitive data like API keys, tokens, or credentials
- Keys and names are unique per session - storing with the same key/name overwrites"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(KvStoreTool), Box::new(SecretStoreTool)]
    }
}

// ============================================================================
// KvStoreTool - Unified key/value storage tool
// ============================================================================

/// Tool for key/value storage operations
pub struct KvStoreTool;

#[async_trait]
impl Tool for KvStoreTool {
    fn name(&self) -> &str {
        "kv_store"
    }

    fn description(&self) -> &str {
        "Key/value storage operations: set, get, delete, or list keys."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["set", "get", "delete", "list"],
                    "description": "The operation to perform"
                },
                "key": {
                    "type": "string",
                    "description": "The key (required for set, get, delete; max 255 chars)"
                },
                "value": {
                    "type": "string",
                    "description": "The value to store (required for set; can be JSON-encoded)"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "kv_store requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let operation = match arguments.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: operation");
            }
        };

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match operation {
            "set" => {
                let key = match arguments.get("key").and_then(|v| v.as_str()) {
                    Some(k) => k,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: key (for set operation)",
                        );
                    }
                };
                let value = match arguments.get("value").and_then(|v| v.as_str()) {
                    Some(v) => v,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: value (for set operation)",
                        );
                    }
                };
                if key.len() > 255 {
                    return ToolExecutionResult::tool_error("Key must be 255 characters or less");
                }
                match storage_store
                    .set_value(context.session_id, key, value)
                    .await
                {
                    Ok(()) => ToolExecutionResult::success(json!({
                        "operation": "set",
                        "key": key,
                        "success": true
                    })),
                    Err(e) => ToolExecutionResult::internal_error(e),
                }
            }
            "get" => {
                let key = match arguments.get("key").and_then(|v| v.as_str()) {
                    Some(k) => k,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: key (for get operation)",
                        );
                    }
                };
                match storage_store.get_value(context.session_id, key).await {
                    Ok(Some(value)) => ToolExecutionResult::success(json!({
                        "operation": "get",
                        "key": key,
                        "value": value,
                        "found": true
                    })),
                    Ok(None) => ToolExecutionResult::success(json!({
                        "operation": "get",
                        "key": key,
                        "value": null,
                        "found": false
                    })),
                    Err(e) => ToolExecutionResult::internal_error(e),
                }
            }
            "delete" => {
                let key = match arguments.get("key").and_then(|v| v.as_str()) {
                    Some(k) => k,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: key (for delete operation)",
                        );
                    }
                };
                match storage_store.delete_value(context.session_id, key).await {
                    Ok(deleted) => ToolExecutionResult::success(json!({
                        "operation": "delete",
                        "key": key,
                        "deleted": deleted
                    })),
                    Err(e) => ToolExecutionResult::internal_error(e),
                }
            }
            "list" => match storage_store.list_keys(context.session_id).await {
                Ok(keys) => {
                    let key_list: Vec<Value> = keys
                        .iter()
                        .map(|k| {
                            json!({
                                "key": k.key,
                                "created_at": k.created_at.to_rfc3339(),
                                "updated_at": k.updated_at.to_rfc3339()
                            })
                        })
                        .collect();
                    ToolExecutionResult::success(json!({
                        "operation": "list",
                        "keys": key_list,
                        "count": key_list.len()
                    }))
                }
                Err(e) => ToolExecutionResult::internal_error(e),
            },
            _ => ToolExecutionResult::tool_error(format!(
                "Invalid operation: {}. Must be one of: set, get, delete, list",
                operation
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SecretStoreTool - Unified secret storage tool
// ============================================================================

/// Tool for encrypted secret storage operations
pub struct SecretStoreTool;

#[async_trait]
impl Tool for SecretStoreTool {
    fn name(&self) -> &str {
        "secret_store"
    }

    fn description(&self) -> &str {
        "Encrypted secret storage operations: set, get, delete, or list secrets."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["set", "get", "delete", "list"],
                    "description": "The operation to perform"
                },
                "name": {
                    "type": "string",
                    "description": "The secret name (required for set, get, delete; max 255 chars)"
                },
                "value": {
                    "type": "string",
                    "description": "The secret value to store (required for set; will be encrypted)"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "secret_store requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let operation = match arguments.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: operation");
            }
        };

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match operation {
            "set" => {
                let name = match arguments.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: name (for set operation)",
                        );
                    }
                };
                let value = match arguments.get("value").and_then(|v| v.as_str()) {
                    Some(v) => v,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: value (for set operation)",
                        );
                    }
                };
                if name.len() > 255 {
                    return ToolExecutionResult::tool_error(
                        "Secret name must be 255 characters or less",
                    );
                }
                match storage_store
                    .set_secret(context.session_id, name, value)
                    .await
                {
                    Ok(()) => ToolExecutionResult::success(json!({
                        "operation": "set",
                        "name": name,
                        "success": true
                    })),
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("Encryption not configured") {
                            ToolExecutionResult::tool_error(
                                "Secret storage not available. Encryption is not configured.",
                            )
                        } else {
                            ToolExecutionResult::internal_error(e)
                        }
                    }
                }
            }
            "get" => {
                let name = match arguments.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: name (for get operation)",
                        );
                    }
                };
                match storage_store.get_secret(context.session_id, name).await {
                    Ok(Some(value)) => ToolExecutionResult::success(json!({
                        "operation": "get",
                        "name": name,
                        "value": value,
                        "found": true
                    })),
                    Ok(None) => ToolExecutionResult::success(json!({
                        "operation": "get",
                        "name": name,
                        "value": null,
                        "found": false
                    })),
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("Encryption not configured") {
                            ToolExecutionResult::tool_error(
                                "Secret storage not available. Encryption is not configured.",
                            )
                        } else {
                            ToolExecutionResult::internal_error(e)
                        }
                    }
                }
            }
            "delete" => {
                let name = match arguments.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Missing required parameter: name (for delete operation)",
                        );
                    }
                };
                match storage_store.delete_secret(context.session_id, name).await {
                    Ok(deleted) => ToolExecutionResult::success(json!({
                        "operation": "delete",
                        "name": name,
                        "deleted": deleted
                    })),
                    Err(e) => ToolExecutionResult::internal_error(e),
                }
            }
            "list" => match storage_store.list_secrets(context.session_id).await {
                Ok(secrets) => {
                    let secret_list: Vec<Value> = secrets
                        .iter()
                        .map(|s| {
                            json!({
                                "name": s.name,
                                "created_at": s.created_at.to_rfc3339(),
                                "updated_at": s.updated_at.to_rfc3339()
                            })
                        })
                        .collect();
                    ToolExecutionResult::success(json!({
                        "operation": "list",
                        "secrets": secret_list,
                        "count": secret_list.len()
                    }))
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("Encryption not configured") {
                        ToolExecutionResult::tool_error(
                            "Secret storage not available. Encryption is not configured.",
                        )
                    } else {
                        ToolExecutionResult::internal_error(e)
                    }
                }
            },
            _ => ToolExecutionResult::tool_error(format!(
                "Invalid operation: {}. Must be one of: set, get, delete, list",
                operation
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = SessionStorageCapability;
        assert_eq!(cap.id(), "session_storage");
        assert_eq!(cap.name(), "Session Storage");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("database"));
        assert_eq!(cap.category(), Some("Storage"));
    }

    #[test]
    fn test_capability_has_two_tools() {
        let cap = SessionStorageCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 2);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"kv_store"));
        assert!(tool_names.contains(&"secret_store"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SessionStorageCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("kv_store"));
        assert!(prompt.contains("secret_store"));
        assert!(prompt.contains("encrypted at rest"));
    }

    #[test]
    fn test_tools_require_context() {
        assert!(KvStoreTool.requires_context());
        assert!(SecretStoreTool.requires_context());
    }

    #[tokio::test]
    async fn test_kv_store_without_context() {
        let tool = KvStoreTool;
        let result = tool
            .execute(json!({"operation": "set", "key": "test", "value": "data"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_kv_store_missing_operation() {
        let tool = KvStoreTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool
            .execute_with_context(json!({"key": "test"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("operation"));
        } else {
            panic!("Expected tool error for missing operation");
        }
    }

    #[tokio::test]
    async fn test_kv_store_no_storage_store() {
        let tool = KvStoreTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool
            .execute_with_context(
                json!({"operation": "set", "key": "test", "value": "data"}),
                &context,
            )
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected tool error for missing storage store");
        }
    }

    #[tokio::test]
    async fn test_secret_store_without_context() {
        let tool = SecretStoreTool;
        let result = tool
            .execute(json!({"operation": "set", "name": "api_key", "value": "secret123"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }
}
