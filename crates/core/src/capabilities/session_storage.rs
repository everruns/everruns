//! Session Storage Capability
//!
//! This capability provides tools for session-scoped key/value and secret storage.
//! Data persists for the session duration.
//!
//! Tools provided:
//! - Key/Value (plain text):
//!   - `set_value`: Store a key/value pair
//!   - `get_value`: Retrieve a value by key
//!   - `delete_value`: Delete a key/value pair
//!   - `list_keys`: List all stored keys
//!
//! - Secrets (encrypted at rest):
//!   - `set_secret`: Store an encrypted secret
//!   - `get_secret`: Retrieve a decrypted secret
//!   - `delete_secret`: Delete a secret
//!   - `list_secrets`: List all secret names

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Session Storage capability - provides key/value and secret storage for sessions
pub struct SessionStorageCapability;

impl Capability for SessionStorageCapability {
    fn id(&self) -> &str {
        CapabilityId::SESSION_STORAGE
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

Key/Value Storage (plain text):
- `set_value`: Store a key/value pair (creates or updates)
- `get_value`: Retrieve a value by key
- `delete_value`: Delete a key/value pair
- `list_keys`: List all stored keys

Secret Storage (encrypted at rest):
- `set_secret`: Store a secret securely (creates or updates)
- `get_secret`: Retrieve a secret (decrypted)
- `delete_secret`: Delete a secret
- `list_secrets`: List all secret names (without values)

Best practices:
- Use key/value storage for general data like state, preferences, or intermediate results
- Use secret storage for sensitive data like API keys, tokens, or credentials
- Keys and secret names are unique per session - storing with the same name overwrites"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            // Key/Value tools
            Box::new(SetValueTool),
            Box::new(GetValueTool),
            Box::new(DeleteValueTool),
            Box::new(ListKeysTool),
            // Secret tools
            Box::new(SetSecretTool),
            Box::new(GetSecretTool),
            Box::new(DeleteSecretTool),
            Box::new(ListSecretsTool),
        ]
    }
}

// ============================================================================
// Key/Value Tools
// ============================================================================

/// Tool to set a key/value pair
pub struct SetValueTool;

#[async_trait]
impl Tool for SetValueTool {
    fn name(&self) -> &str {
        "set_value"
    }

    fn description(&self) -> &str {
        "Store a key/value pair. Creates a new entry or updates an existing one."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The key to store the value under (max 255 chars)"
                },
                "value": {
                    "type": "string",
                    "description": "The value to store (can be JSON-encoded for structured data)"
                }
            },
            "required": ["key", "value"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "set_value requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let key = match arguments.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => return ToolExecutionResult::tool_error("Missing required parameter: key"),
        };

        let value = match arguments.get("value").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return ToolExecutionResult::tool_error("Missing required parameter: value"),
        };

        if key.len() > 255 {
            return ToolExecutionResult::tool_error("Key must be 255 characters or less");
        }

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store
            .set_value(context.session_id, key, value)
            .await
        {
            Ok(()) => ToolExecutionResult::success(json!({
                "key": key,
                "stored": true
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Tool to get a value by key
pub struct GetValueTool;

#[async_trait]
impl Tool for GetValueTool {
    fn name(&self) -> &str {
        "get_value"
    }

    fn description(&self) -> &str {
        "Retrieve a stored value by its key. Returns null if the key doesn't exist."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The key to retrieve the value for"
                }
            },
            "required": ["key"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "get_value requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let key = match arguments.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => return ToolExecutionResult::tool_error("Missing required parameter: key"),
        };

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store.get_value(context.session_id, key).await {
            Ok(Some(value)) => ToolExecutionResult::success(json!({
                "key": key,
                "value": value,
                "found": true
            })),
            Ok(None) => ToolExecutionResult::success(json!({
                "key": key,
                "value": null,
                "found": false
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Tool to delete a key/value pair
pub struct DeleteValueTool;

#[async_trait]
impl Tool for DeleteValueTool {
    fn name(&self) -> &str {
        "delete_value"
    }

    fn description(&self) -> &str {
        "Delete a stored key/value pair. Returns whether the key existed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The key to delete"
                }
            },
            "required": ["key"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "delete_value requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let key = match arguments.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => return ToolExecutionResult::tool_error("Missing required parameter: key"),
        };

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store.delete_value(context.session_id, key).await {
            Ok(deleted) => ToolExecutionResult::success(json!({
                "key": key,
                "deleted": deleted
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Tool to list all stored keys
pub struct ListKeysTool;

#[async_trait]
impl Tool for ListKeysTool {
    fn name(&self) -> &str {
        "list_keys"
    }

    fn description(&self) -> &str {
        "List all stored keys in the session (without their values)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "list_keys requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store.list_keys(context.session_id).await {
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
                    "keys": key_list,
                    "count": key_list.len()
                }))
            }
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Secret Tools
// ============================================================================

/// Tool to set a secret
pub struct SetSecretTool;

#[async_trait]
impl Tool for SetSecretTool {
    fn name(&self) -> &str {
        "set_secret"
    }

    fn description(&self) -> &str {
        "Store a secret securely. The value is encrypted at rest. Creates or updates."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name/identifier for the secret (max 255 chars)"
                },
                "value": {
                    "type": "string",
                    "description": "The secret value to store (will be encrypted)"
                }
            },
            "required": ["name", "value"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "set_secret requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolExecutionResult::tool_error("Missing required parameter: name"),
        };

        let value = match arguments.get("value").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return ToolExecutionResult::tool_error("Missing required parameter: value"),
        };

        if name.len() > 255 {
            return ToolExecutionResult::tool_error("Secret name must be 255 characters or less");
        }

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store
            .set_secret(context.session_id, name, value)
            .await
        {
            Ok(()) => ToolExecutionResult::success(json!({
                "name": name,
                "stored": true
            })),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Encryption not configured") {
                    ToolExecutionResult::tool_error(
                        "Secret storage is not available. Encryption is not configured.",
                    )
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Tool to get a secret
pub struct GetSecretTool;

#[async_trait]
impl Tool for GetSecretTool {
    fn name(&self) -> &str {
        "get_secret"
    }

    fn description(&self) -> &str {
        "Retrieve a stored secret by name. The value is decrypted before returning."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the secret to retrieve"
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "get_secret requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolExecutionResult::tool_error("Missing required parameter: name"),
        };

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store.get_secret(context.session_id, name).await {
            Ok(Some(value)) => ToolExecutionResult::success(json!({
                "name": name,
                "value": value,
                "found": true
            })),
            Ok(None) => ToolExecutionResult::success(json!({
                "name": name,
                "value": null,
                "found": false
            })),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Encryption not configured") {
                    ToolExecutionResult::tool_error(
                        "Secret storage is not available. Encryption is not configured.",
                    )
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Tool to delete a secret
pub struct DeleteSecretTool;

#[async_trait]
impl Tool for DeleteSecretTool {
    fn name(&self) -> &str {
        "delete_secret"
    }

    fn description(&self) -> &str {
        "Delete a stored secret. Returns whether the secret existed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the secret to delete"
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "delete_secret requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolExecutionResult::tool_error("Missing required parameter: name"),
        };

        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store.delete_secret(context.session_id, name).await {
            Ok(deleted) => ToolExecutionResult::success(json!({
                "name": name,
                "deleted": deleted
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Tool to list all secret names
pub struct ListSecretsTool;

#[async_trait]
impl Tool for ListSecretsTool {
    fn name(&self) -> &str {
        "list_secrets"
    }

    fn description(&self) -> &str {
        "List all stored secret names in the session (without their values)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "list_secrets requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let storage_store = match &context.storage_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("Storage not available in this context");
            }
        };

        match storage_store.list_secrets(context.session_id).await {
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
                    "secrets": secret_list,
                    "count": secret_list.len()
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Encryption not configured") {
                    ToolExecutionResult::tool_error(
                        "Secret storage is not available. Encryption is not configured.",
                    )
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
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
        assert_eq!(cap.id(), CapabilityId::SESSION_STORAGE);
        assert_eq!(cap.name(), "Session Storage");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("database"));
        assert_eq!(cap.category(), Some("Storage"));
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = SessionStorageCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 8);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        // Key/Value tools
        assert!(tool_names.contains(&"set_value"));
        assert!(tool_names.contains(&"get_value"));
        assert!(tool_names.contains(&"delete_value"));
        assert!(tool_names.contains(&"list_keys"));
        // Secret tools
        assert!(tool_names.contains(&"set_secret"));
        assert!(tool_names.contains(&"get_secret"));
        assert!(tool_names.contains(&"delete_secret"));
        assert!(tool_names.contains(&"list_secrets"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SessionStorageCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("set_value"));
        assert!(prompt.contains("get_value"));
        assert!(prompt.contains("set_secret"));
        assert!(prompt.contains("encrypted at rest"));
    }

    #[test]
    fn test_tools_require_context() {
        assert!(SetValueTool.requires_context());
        assert!(GetValueTool.requires_context());
        assert!(DeleteValueTool.requires_context());
        assert!(ListKeysTool.requires_context());
        assert!(SetSecretTool.requires_context());
        assert!(GetSecretTool.requires_context());
        assert!(DeleteSecretTool.requires_context());
        assert!(ListSecretsTool.requires_context());
    }

    #[tokio::test]
    async fn test_set_value_without_context() {
        let tool = SetValueTool;
        let result = tool.execute(json!({"key": "test", "value": "data"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_set_value_missing_key() {
        let tool = SetValueTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool
            .execute_with_context(json!({"value": "data"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing key");
        }
    }

    #[tokio::test]
    async fn test_set_value_no_storage_store() {
        let tool = SetValueTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool
            .execute_with_context(json!({"key": "test", "value": "data"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected tool error for missing storage store");
        }
    }

    #[tokio::test]
    async fn test_set_secret_without_context() {
        let tool = SetSecretTool;
        let result = tool
            .execute(json!({"name": "api_key", "value": "secret123"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }
}
