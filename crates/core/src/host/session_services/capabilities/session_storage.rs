//! Session storage capability.
//!
//! This capability provides tools for session-scoped key/value and secret storage.
//! Data persists for the session duration.
//!
//! Tools provided:
//! - `kv_store`: Key/value storage operations (set, get, delete, list)
//! - `secret_store`: Encrypted secret storage operations (set, get, delete, list)

use crate::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::tool_context::ToolContext;
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use serde_json::{Value, json};

// Reserve internal KV prefixes from the user-facing kv_store. Reference the
// canonical constants so these never drift from how the owning capabilities
// write them: A2A run records (`a2a_delegation::run_key`) and ARD runtime
// attachments / discovery cache (`ard_attachment`). Reserving the ARD prefixes
// stops a session/tool actor forging attachments via kv_store (TM-TOOL/TM-AGENT).
const INTERNAL_KV_PREFIXES: &[&str] = &[
    crate::capabilities::AGENT_RUN_KEY_PREFIX,
    crate::ard_attachment::ARD_ATTACHMENT_KV_PREFIX,
    crate::ard_attachment::ARD_DISCOVERY_KV_PREFIX,
    // Session MCP servers (ARD MCP targets, chat-only user servers): a session
    // actor that could write one would add any MCP server it liked.
    crate::session_mcp_servers::SESSION_MCP_SERVER_KV_PREFIX,
    // Revealed deferred MCP servers: written only by tool search and the
    // server's placeholder, so the kv_store tool cannot list or clear them.
    crate::DEFERRED_MCP_REVEAL_KV_PREFIX,
    // Persisted channel ThreadContext (EVE-977). Reserved for the same reason
    // as the ARD prefixes: its participant list and "user is viewing" hint
    // reach the model as context, so a session/tool actor forging them would
    // be writing its own prompt.
    crate::channel::THREAD_CONTEXT_KV_KEY,
    // Durable tool-approval decisions (EVE-1140). THREAT[TM-TOOL-008]: a
    // session/tool actor that could write here would approve its own calls.
    crate::capabilities::TOOL_APPROVAL_KV_PREFIX,
    // MCP elicitation consent and form answers (EVE-1141). THREAT[TM-TOOL-034]:
    // the retried tool call honours whatever record sits here, so a model that
    // could write one would accept a URL elicitation or answer a server's form
    // on the person's behalf. Only the answer APIs write these.
    crate::capabilities::MCP_ELICITATION_CONSENT_KV_PREFIX,
    crate::capabilities::MCP_ELICITATION_FORM_KV_PREFIX,
    // Computer-use action counter (EVE-1141). THREAT[TM-TOOL-050]: resetting or
    // deleting it would lift the per-session action cap.
    crate::computer_use::COMPUTER_USE_ACTION_COUNT_KEY,
    // Which sandbox hosts a session's computer-use desktop (EVE-1133).
    crate::computer_use::COMPUTER_USE_DISPLAY_KV_PREFIX,
    // Which native batch failed; a model that could write it would skip or
    // unskip the rest of a batch.
    crate::computer_use::COMPUTER_USE_FAILED_BATCH_KEY,
    // Browser-use bookkeeping: the action counter (THREAT[TM-TOOL-050]), the
    // element refs a click resolves, the active and known tabs.
    crate::browser_use::BROWSER_USE_KV_PREFIX,
];
// Capability-owned secret namespaces. Literals are repeated from the owning
// crates (platform / integrations) because host must not import those crates
// (check-agent-record-isolation.sh). Each owning crate pins its constant to
// these reservations with a unit test beside the definition.
//
// THREAT[TM-SANDBOX-004] / THREAT[TM-AGENT-020]: session actors must not
// overwrite sandbox state secrets via the user-facing secret_store; those
// records carry provider resource IDs that tools then act on.
const INTERNAL_SECRET_PREFIXES: &[&str] = &[
    "browserless_internal:",
    "mcp_oauth:",
    "container_sandbox:",
    "daytona_sandbox:",
    "e2b_sandbox:",
    "deno_sandbox:",
    "sprites_sprite:",
    "modal_sandbox:",
];
// Exact reserved secret names. Unlike the prefixes above, this one cannot
// reference its canonical constant: SESSION_SANDBOX_SECRET_NAME is defined in
// the platform crate, which depends on this one and which host source must not
// reference (check-agent-record-isolation.sh). The name is repeated here and
// pinned to the constant by a test beside that definition.
const INTERNAL_SECRET_NAMES: &[&str] = &["session_sandbox"];

pub fn is_internal_session_kv_key(key: &str) -> bool {
    INTERNAL_KV_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

fn reserved_kv_key_error() -> ToolExecutionResult {
    ToolExecutionResult::tool_error("Key is reserved for internal system use")
}

pub fn is_internal_session_secret_name(name: &str) -> bool {
    INTERNAL_SECRET_NAMES.contains(&name)
        || INTERNAL_SECRET_PREFIXES
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
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Key-Value Store");
        Some(crate::tool_narration::narrate_secret_store(
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
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Secret Store");
        Some(crate::tool_narration::narrate_secret_store(
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
    use crate::session_services::KeyInfo;
    use crate::session_services::SessionStorageStore;
    use everruns_contracts::error::Result;
    use everruns_contracts::typed_id::SessionId;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct TestStorageStore {
        values: Mutex<HashMap<String, String>>,
    }

    #[async_trait]
    impl crate::session_services::SessionStorageStore for TestStorageStore {
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
        ) -> Result<Vec<crate::session_services::SecretInfo>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn test_internal_kv_key_filtering() {
        assert!(is_internal_session_kv_key("agent_run:abc"));
        assert!(is_internal_session_kv_key("session_mcp:docs"));
        assert!(is_internal_session_kv_key(
            "tool_approval/always/send_email"
        ));
        assert!(is_internal_session_kv_key("tool_approval/once/sha256_ab"));
        assert!(is_internal_session_kv_key(
            "mcp/elicitation-consent/billing/charge"
        ));
        assert!(is_internal_session_kv_key(
            "mcp/elicitation-form/deploys/release"
        ));
        assert!(is_internal_session_kv_key("computer_use.action_count"));
        assert!(is_internal_session_kv_key("computer_use.failed_batch"));
        assert!(is_internal_session_kv_key("browser_use.action_count"));
        assert!(is_internal_session_kv_key("browser_use.refs.T1"));
        assert!(!is_internal_session_kv_key("user:agent_run:abc"));
        assert!(!is_internal_session_kv_key("mcp/notes"));
        assert!(!is_internal_session_kv_key("computer_use.notes"));
    }

    // EVE-1141: every gate whose decision is read back from session storage
    // must be out of the model's reach through kv_store, for every operation.
    #[tokio::test]
    async fn test_kv_store_rejects_every_security_gate_key() {
        let tool = KvStoreTool;
        let session_id = SessionId::new();
        let storage = Arc::new(TestStorageStore::default());
        let context = ToolContext::with_storage_store(session_id, storage.clone());
        let gate_keys = [
            "tool_approval/always/send_email",
            "mcp/elicitation-consent/billing/charge",
            "mcp/elicitation-form/deploys/release",
            crate::computer_use::COMPUTER_USE_ACTION_COUNT_KEY,
            "computer_use.display.e2b",
            "computer_use.display.daytona",
            crate::computer_use::COMPUTER_USE_FAILED_BATCH_KEY,
        ];

        for key in gate_keys {
            storage.set_value(session_id, key, "trusted").await.unwrap();
            for arguments in [
                json!({"operation": "set", "key": key, "value": "forged"}),
                json!({"operation": "get", "key": key}),
                json!({"operation": "delete", "key": key}),
            ] {
                let result = tool.execute_with_context(arguments, &context).await;
                assert!(
                    matches!(result, ToolExecutionResult::ToolError(ref msg) if msg.contains("reserved")),
                    "{key}: expected reserved-key error, got {result:?}"
                );
            }
            assert_eq!(
                storage.get_value(session_id, key).await.unwrap().as_deref(),
                Some("trusted"),
                "{key}: the record the system wrote is untouched"
            );
        }

        let ToolExecutionResult::Success(listed) = tool
            .execute_with_context(json!({"operation": "list"}), &context)
            .await
        else {
            panic!("expected successful list");
        };
        assert_eq!(listed["count"], 0, "no gate key is listed: {listed}");
    }

    #[test]
    fn test_internal_secret_name_filtering() {
        assert!(is_internal_session_secret_name(
            "browserless_internal:cookies"
        ));
        assert!(is_internal_session_secret_name(
            "mcp_oauth:server:access_token"
        ));
        assert!(is_internal_session_secret_name(
            "container_sandbox:evr-deadbeef-sandbox"
        ));
        assert!(is_internal_session_secret_name("daytona_sandbox:sbx-123"));
        assert!(is_internal_session_secret_name("e2b_sandbox:i-abc"));
        assert!(is_internal_session_secret_name("deno_sandbox:sb_1"));
        assert!(is_internal_session_secret_name("sprites_sprite:sprite-1"));
        assert!(is_internal_session_secret_name("modal_sandbox:sb-1"));
        assert!(is_internal_session_secret_name("session_sandbox"));
        assert!(!is_internal_session_secret_name("api_key"));
        assert!(!is_internal_session_secret_name("container_sandbox"));
        assert!(!is_internal_session_secret_name("daytona_sandbox"));
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

    // EVE-1151: capability-owned sandbox state namespaces stay out of secret_store.
    #[tokio::test]
    async fn test_secret_store_rejects_reserved_sandbox_prefixes() {
        #[derive(Default)]
        struct SecretMemory {
            secrets: Mutex<HashMap<String, String>>,
        }

        #[async_trait]
        impl SessionStorageStore for SecretMemory {
            async fn set_value(
                &self,
                _session_id: SessionId,
                _key: &str,
                _value: &str,
            ) -> Result<()> {
                Ok(())
            }
            async fn get_value(
                &self,
                _session_id: SessionId,
                _key: &str,
            ) -> Result<Option<String>> {
                Ok(None)
            }
            async fn delete_value(&self, _session_id: SessionId, _key: &str) -> Result<bool> {
                Ok(false)
            }
            async fn list_keys(&self, _session_id: SessionId) -> Result<Vec<KeyInfo>> {
                Ok(vec![])
            }
            async fn set_secret(
                &self,
                _session_id: SessionId,
                name: &str,
                value: &str,
            ) -> Result<()> {
                self.secrets
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), value.to_string());
                Ok(())
            }
            async fn get_secret(
                &self,
                _session_id: SessionId,
                name: &str,
            ) -> Result<Option<String>> {
                Ok(self.secrets.lock().unwrap().get(name).cloned())
            }
            async fn delete_secret(&self, _session_id: SessionId, name: &str) -> Result<bool> {
                Ok(self.secrets.lock().unwrap().remove(name).is_some())
            }
            async fn list_secrets(
                &self,
                _session_id: SessionId,
            ) -> Result<Vec<crate::session_services::SecretInfo>> {
                let now = chrono::Utc::now();
                Ok(self
                    .secrets
                    .lock()
                    .unwrap()
                    .keys()
                    .map(|name| crate::session_services::SecretInfo {
                        name: name.clone(),
                        created_at: now,
                        updated_at: now,
                    })
                    .collect())
            }
        }

        let tool = SecretStoreTool;
        let session_id = SessionId::new();
        let storage = Arc::new(SecretMemory::default());
        // System-written trusted records (bypass the tool).
        for name in [
            "container_sandbox:evr-owned-sandbox",
            "daytona_sandbox:sbx-1",
            "e2b_sandbox:i-1",
            "deno_sandbox:sb_1",
            "sprites_sprite:sprite-1",
            "modal_sandbox:sb-1",
        ] {
            storage
                .set_secret(session_id, name, "trusted")
                .await
                .unwrap();
        }
        let context = ToolContext::with_storage_store(session_id, storage.clone());

        for name in [
            "container_sandbox:evr-owned-sandbox",
            "daytona_sandbox:sbx-1",
            "e2b_sandbox:i-1",
            "deno_sandbox:sb_1",
            "sprites_sprite:sprite-1",
            "modal_sandbox:sb-1",
        ] {
            let set_result = tool
                .execute_with_context(
                    json!({"operation": "set", "name": name, "value": "forged"}),
                    &context,
                )
                .await;
            assert!(
                matches!(set_result, ToolExecutionResult::ToolError(ref msg) if msg.contains("reserved")),
                "{name}: set must be reserved, got {set_result:?}"
            );

            // get deliberately looks like a miss so reserved names cannot be enumerated
            let get_result = tool
                .execute_with_context(json!({"operation": "get", "name": name}), &context)
                .await;
            assert!(
                matches!(get_result, ToolExecutionResult::ToolError(ref msg) if msg.contains("not found")),
                "{name}: get must look like a miss, got {get_result:?}"
            );

            let delete_result = tool
                .execute_with_context(json!({"operation": "delete", "name": name}), &context)
                .await;
            assert!(
                matches!(delete_result, ToolExecutionResult::ToolError(ref msg) if msg.contains("reserved")),
                "{name}: delete must be reserved, got {delete_result:?}"
            );

            assert_eq!(
                storage
                    .get_secret(session_id, name)
                    .await
                    .unwrap()
                    .as_deref(),
                Some("trusted"),
                "{name}: system-written secret must stay untouched"
            );
        }

        let ToolExecutionResult::Success(listed) = tool
            .execute_with_context(json!({"operation": "list"}), &context)
            .await
        else {
            panic!("expected successful list");
        };
        assert_eq!(
            listed["count"], 0,
            "reserved sandbox secrets must not be listed: {listed}"
        );
    }
}
