//! Session Storage Capability
//!
//! This capability provides tools for session-scoped key/value and secret storage.
//! Data persists for the session duration.
//!
//! Tools provided:
//! - `kv_store`: Key/value storage operations (set, get, delete, list)
//! - `secret_store`: Encrypted secret storage operations (set, get, delete, list)

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};

// Reserve internal KV prefixes from the user-facing kv_store. Reference the
// canonical constants so these never drift from how the owning capabilities
// write them: A2A run records (`a2a_delegation::run_key`) and ARD runtime
// attachments / discovery cache (`ard_attachment`). Reserving the ARD prefixes
// stops a session/tool actor forging attachments via kv_store (TM-TOOL/TM-AGENT).
const INTERNAL_KV_PREFIXES: &[&str] = &[
    everruns_core::capabilities::AGENT_RUN_KEY_PREFIX,
    everruns_core::ard_attachment::ARD_ATTACHMENT_KV_PREFIX,
    everruns_core::ard_attachment::ARD_DISCOVERY_KV_PREFIX,
];
const INTERNAL_SECRET_PREFIXES: &[&str] = &["browserless_internal:", "mcp_oauth:"];

pub fn is_internal_session_kv_key(key: &str) -> bool {
    INTERNAL_KV_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

fn reserved_kv_key_error() -> ToolExecutionResult {
    ToolExecutionResult::tool_error("Key is reserved for internal system use")
}

pub fn is_internal_session_secret_name(name: &str) -> bool {
    INTERNAL_SECRET_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

pub const SESSION_STORAGE_CAPABILITY_ID: &str = "session_storage";

/// Session Storage capability - provides key/value and secret storage for sessions
pub struct SessionStorageCapability;

impl Capability for SessionStorageCapability {
    fn id(&self) -> &str {
        SESSION_STORAGE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Storage"
    }

    fn description(&self) -> &str {
        r#"Tools to store and retrieve key/value pairs and encrypted secrets within a session.

> [!NOTE]
> Data persists for the session duration. Secrets are encrypted at rest.

> [!TIP]
> Use key/value storage for general data. Use secrets for sensitive information like API keys or tokens."#
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Сховище",
            r#"Інструменти для збереження та отримання пар ключ-значення і зашифрованих секретів у межах сесії.

> [!NOTE]
> Дані зберігаються протягом усієї сесії. Секрети шифруються при зберіганні.

> [!TIP]
> Використовуйте сховище ключ-значення для загальних даних. Використовуйте секрети для чутливої інформації, як-от API-ключі чи токени."#,
        )]
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
            "Use `kv_store` for general data. Use `secret_store` for sensitive data (API keys, tokens, credentials) — secrets are encrypted at rest. Keys are unique per session; storing with the same key overwrites.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(KvStoreTool), Box::new(SecretStoreTool)]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["secrets", "key_value"]
    }
}

// ============================================================================
// KvStoreTool - Unified key/value storage tool
// ============================================================================

/// Tool for key/value storage operations
pub struct KvStoreTool;

#[async_trait]
impl Tool for KvStoreTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Key-Value Store");
        Some(everruns_core::tool_narration::narrate_secret_store(
            &tool_call.arguments,
            fallback,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "kv_store"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Key-Value Store")
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

    fn hints(&self) -> ToolHints {
        // Mutates shared session storage on set/delete; serialize storage
        // mutations within a batch to avoid lost updates.
        ToolHints::default()
            .with_idempotent(true)
            .with_concurrency_class("session_storage")
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
                if is_internal_session_kv_key(key) {
                    return reserved_kv_key_error();
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
                if is_internal_session_kv_key(key) {
                    return reserved_kv_key_error();
                }
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
                if is_internal_session_kv_key(key) {
                    return reserved_kv_key_error();
                }
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
                        .filter(|k| !is_internal_session_kv_key(&k.key))
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
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Secret Store");
        Some(everruns_core::tool_narration::narrate_secret_store(
            &tool_call.arguments,
            fallback,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "secret_store"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Secret Store")
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

    fn hints(&self) -> ToolHints {
        // Shares the session storage backend with kv_store; serialize storage
        // mutations within a batch to avoid lost updates.
        ToolHints::default()
            .with_idempotent(true)
            .with_requires_secrets(true)
            .with_concurrency_class("session_storage")
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
                if is_internal_session_secret_name(name) {
                    return ToolExecutionResult::tool_error(
                        "Secret name is reserved for internal system use",
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
                if is_internal_session_secret_name(name) {
                    return ToolExecutionResult::tool_error("Secret not found");
                }
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
                if is_internal_session_secret_name(name) {
                    return ToolExecutionResult::tool_error(
                        "Secret name is reserved for internal system use",
                    );
                }
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
                        .filter(|s| !is_internal_session_secret_name(&s.name))
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
    use everruns_core::traits::SessionStorageStore;
    use everruns_core::typed_id::SessionId;
    use everruns_core::{KeyInfo, Result};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct TestStorageStore {
        values: Mutex<HashMap<String, String>>,
    }

    #[async_trait]
    impl everruns_core::traits::SessionStorageStore for TestStorageStore {
        async fn set_value(&self, _session_id: SessionId, key: &str, value: &str) -> Result<()> {
            self.values
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        async fn get_value(&self, _session_id: SessionId, key: &str) -> Result<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        async fn delete_value(&self, _session_id: SessionId, key: &str) -> Result<bool> {
            Ok(self.values.lock().unwrap().remove(key).is_some())
        }

        async fn list_keys(&self, _session_id: SessionId) -> Result<Vec<KeyInfo>> {
            let now = chrono::Utc::now();
            Ok(self
                .values
                .lock()
                .unwrap()
                .keys()
                .map(|key| KeyInfo {
                    key: key.clone(),
                    created_at: now,
                    updated_at: now,
                })
                .collect())
        }

        async fn set_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
            _value: &str,
        ) -> Result<()> {
            Ok(())
        }

        async fn get_secret(&self, _session_id: SessionId, _name: &str) -> Result<Option<String>> {
            Ok(None)
        }

        async fn delete_secret(&self, _session_id: SessionId, _name: &str) -> Result<bool> {
            Ok(false)
        }

        async fn list_secrets(
            &self,
            _session_id: SessionId,
        ) -> Result<Vec<everruns_core::SecretInfo>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn test_internal_kv_key_filtering() {
        assert!(is_internal_session_kv_key("agent_run:abc"));
        assert!(!is_internal_session_kv_key("user:agent_run:abc"));
    }

    #[test]
    fn test_internal_secret_name_filtering() {
        assert!(is_internal_session_secret_name(
            "browserless_internal:cookies"
        ));
        assert!(is_internal_session_secret_name(
            "mcp_oauth:server:access_token"
        ));
        assert!(!is_internal_session_secret_name("api_key"));
    }

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SessionStorageCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("kv_store"));
        assert!(prompt.contains("secret_store"));
        assert!(prompt.contains("encrypted"));
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
        let context = ToolContext::new(SessionId::new());

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
        let context = ToolContext::new(SessionId::new());

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
    async fn test_kv_store_rejects_reserved_internal_keys() {
        let tool = KvStoreTool;
        let session_id = SessionId::new();
        let storage = Arc::new(TestStorageStore::default());
        storage
            .set_value(session_id, "agent_run:trusted", "trusted-record")
            .await
            .unwrap();
        storage
            .set_value(session_id, "public", "public-record")
            .await
            .unwrap();
        let context = ToolContext::with_storage_store(session_id, storage.clone());

        for arguments in [
            json!({"operation": "set", "key": "agent_run:trusted", "value": "forged"}),
            json!({"operation": "get", "key": "agent_run:trusted"}),
            json!({"operation": "delete", "key": "agent_run:trusted"}),
        ] {
            let result = tool.execute_with_context(arguments, &context).await;
            assert!(
                matches!(result, ToolExecutionResult::ToolError(ref msg) if msg.contains("reserved")),
                "expected reserved-key error, got {result:?}"
            );
        }

        assert_eq!(
            storage
                .get_value(session_id, "agent_run:trusted")
                .await
                .unwrap()
                .as_deref(),
            Some("trusted-record")
        );

        let result = tool
            .execute_with_context(json!({"operation": "list"}), &context)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected successful list");
        };
        assert_eq!(value["count"], 1);
        assert_eq!(value["keys"][0]["key"], "public");
    }

    #[tokio::test]
    async fn test_secret_store_without_context() {
        let tool = SecretStoreTool;
        let result = tool
            .execute(json!({"operation": "set", "name": "api_key", "value": "YExample0"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }
}
