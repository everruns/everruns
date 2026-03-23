//! Sprites API types and session state management.
//!
//! Decision: Embrace persistence — SpriteState includes the public HTTP URL

use everruns_core::UpsertLeasedResource;
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{error, warn};

use crate::SPRITES_SECRET_PREFIX;

// ============================================================================
// API Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpriteInfo {
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInfo {
    pub id: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

// ============================================================================
// Persisted Sprite State (stored in session secrets as JSON)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpriteState {
    pub sprite_name: String,
    pub workspace_path: String,
    pub started_at: String,
    /// Public HTTP URL for this sprite (if it exposes a service on port 8080)
    #[serde(default)]
    pub service_url: Option<String>,
}

/// Everruns lease duration for Sprites.
///
/// Sprites hibernate when idle (no compute charges), but we still register
/// a lease so the control plane can clean up forgotten sprites.
pub const SPRITES_LEASE_DURATION_SECONDS: u32 = 30 * 60;

// ============================================================================
// State Management Helpers
// ============================================================================

/// Resolve Sprites API token via user connection (Settings > Connections > Sprites).
pub async fn get_api_token(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    if let Some(resolver) = context.connection_resolver.as_ref() {
        match resolver
            .get_connection_token(context.session_id, "sprites")
            .await
        {
            Ok(Some(token)) => return Ok(token),
            Ok(None) => {} // fall through to error
            Err(e) => {
                error!("Failed to resolve Sprites user connection: {e}");
            }
        }
    }

    // THREAT[TM-AGENT-016]: Asking secrets in chat stores them plaintext in events.
    // Return ConnectionRequired so the UI renders an inline connection dialog.
    Err(ToolExecutionResult::connection_required("sprites"))
}

pub async fn get_sprite_state(
    context: &ToolContext,
    sprite_name: &str,
) -> Result<SpriteState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{SPRITES_SECRET_PREFIX}{sprite_name}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read sprite state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sprite state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sprite '{sprite_name}' not found. Create one first with sprites_create_sprite."
            ))
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt sprite state for {sprite_name}: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt sprite state: {e}"))
    })
}

pub async fn save_sprite_state(
    context: &ToolContext,
    state: &SpriteState,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{SPRITES_SECRET_PREFIX}{}", state.sprite_name);
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize sprite state: {e}"))
    })?;

    storage
        .set_secret(context.session_id, &secret_name, &json_str)
        .await
        .map_err(|e| {
            error!("Failed to save sprite state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to save sprite state: {e}"))
        })
}

pub async fn delete_sprite_state(
    context: &ToolContext,
    sprite_name: &str,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{SPRITES_SECRET_PREFIX}{sprite_name}");
    storage
        .delete_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to delete sprite state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete sprite state: {e}"))
        })?;
    Ok(())
}

pub async fn list_sprite_states(
    context: &ToolContext,
) -> Result<Vec<SpriteState>, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secrets = storage
        .list_secrets(context.session_id)
        .await
        .map_err(|e| {
            error!("Failed to list secrets: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to list secrets: {e}"))
        })?;

    let mut states = Vec::new();
    for secret_info in secrets {
        if let Some(sprite_name) = secret_info.name.strip_prefix(SPRITES_SECRET_PREFIX) {
            match get_sprite_state(context, sprite_name).await {
                Ok(state) => states.push(state),
                Err(_) => {
                    warn!("Skipping corrupt sprite state: {}", sprite_name);
                }
            }
        }
    }

    Ok(states)
}

/// Register or refresh the Sprites lease for generic cleanup.
pub async fn touch_sprite_lease(
    context: &ToolContext,
    state: &SpriteState,
    display_name: Option<String>,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    let owner_user_id = if let Some(resolver) = context.connection_resolver.as_ref() {
        resolver
            .get_connection_user(context.session_id, "sprites")
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: "sprites".to_string(),
            resource_type: "sprite".to_string(),
            external_id: state.sprite_name.clone(),
            display_name,
            owner_user_id,
            lease_duration_seconds: SPRITES_LEASE_DURATION_SECONDS,
            // THREAT[TM-API-015]: leased-resource metadata is API-visible.
            // Keep only non-secret fields here. service_url is the sprite's
            // public URL (not a credential) so it's safe to include for UI display.
            metadata: json!({
                "workspace_path": state.workspace_path,
                "started_at": state.started_at,
                "service_url": state.service_url,
            }),
        })
        .await
        .map_err(|e| {
            error!("Failed to upsert Sprites lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to update Sprites lease: {e}"))
        })?;

    Ok(())
}

/// Release the Sprites lease after explicit deletion.
pub async fn release_sprite_lease(
    context: &ToolContext,
    sprite_name: &str,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .release_resource(context.session_id, "sprites", "sprite", sprite_name)
        .await
        .map_err(|e| {
            error!("Failed to release Sprites lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to release Sprites lease: {e}"))
        })?;

    Ok(())
}

/// Extract a required string parameter from tool arguments.
pub fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
    })
}

/// Validate that a sprite name is safe for URL interpolation.
///
/// Sprite names are user-supplied (unlike Daytona sandbox IDs which come from the API).
/// Must be lowercase alphanumeric with hyphens, 1-63 chars, no leading/trailing hyphens.
pub fn validate_sprite_name(name: &str) -> Result<(), ToolExecutionResult> {
    if name.is_empty() || name.len() > 63 {
        return Err(ToolExecutionResult::tool_error(
            "Sprite name must be 1-63 characters",
        ));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(ToolExecutionResult::tool_error(
            "Sprite name must contain only lowercase letters, digits, and hyphens",
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(ToolExecutionResult::tool_error(
            "Sprite name must not start or end with a hyphen",
        ));
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sprite_state_roundtrip() {
        let state = SpriteState {
            sprite_name: "my-sprite".to_string(),
            workspace_path: "/home/user".to_string(),
            started_at: "2026-03-23T10:00:00Z".to_string(),
            service_url: Some("https://my-sprite.fly.dev".to_string()),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SpriteState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sprite_name, "my-sprite");
        assert_eq!(deserialized.workspace_path, "/home/user");
        assert_eq!(
            deserialized.service_url,
            Some("https://my-sprite.fly.dev".to_string())
        );
    }

    #[test]
    fn test_sprite_state_without_url() {
        let state = SpriteState {
            sprite_name: "headless".to_string(),
            workspace_path: "/home/user".to_string(),
            started_at: "2026-03-23T10:00:00Z".to_string(),
            service_url: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SpriteState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.service_url, None);
    }

    #[test]
    fn test_required_str_present() {
        let args = serde_json::json!({"name": "test_value"});
        let result = required_str(&args, "name");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
    }

    #[test]
    fn test_required_str_missing() {
        let args = serde_json::json!({"other": "value"});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_null_value() {
        let args = serde_json::json!({"name": null});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_non_string_value() {
        let args = serde_json::json!({"name": 42});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_sprite_info_deserialization() {
        let json =
            r#"{"name": "my-sprite", "status": "running", "url": "https://my-sprite.fly.dev"}"#;
        let info: SpriteInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.name, "my-sprite");
        assert_eq!(info.status, "running");
        assert_eq!(info.url, Some("https://my-sprite.fly.dev".to_string()));
    }

    #[test]
    fn test_sprite_info_without_url() {
        let json = r#"{"name": "headless", "status": "stopped"}"#;
        let info: SpriteInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.url, None);
    }

    #[test]
    fn test_exec_result_deserialization() {
        let json = r#"{"stdout": "hello\n", "stderr": "", "exit_code": 0}"#;
        let result: ExecResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.stdout, "hello\n");
        assert_eq!(result.stderr, "");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_exec_result_defaults() {
        let json = r#"{}"#;
        let result: ExecResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_checkpoint_info_deserialization() {
        let json = r#"{"id": "cp_abc", "created_at": "2026-03-23T10:00:00Z"}"#;
        let info: CheckpointInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "cp_abc");
        assert_eq!(info.created_at, Some("2026-03-23T10:00:00Z".to_string()));
    }

    #[test]
    fn test_checkpoint_info_without_timestamp() {
        let json = r#"{"id": "cp_xyz"}"#;
        let info: CheckpointInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "cp_xyz");
        assert_eq!(info.created_at, None);
    }

    #[test]
    fn test_sprite_state_json_has_all_fields() {
        let state = SpriteState {
            sprite_name: "sp_x".to_string(),
            workspace_path: "/home/user".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            service_url: None,
        };
        let val: serde_json::Value = serde_json::to_value(&state).unwrap();
        assert!(val.get("sprite_name").is_some());
        assert!(val.get("workspace_path").is_some());
        assert!(val.get("started_at").is_some());
    }

    #[test]
    fn test_validate_sprite_name_valid() {
        assert!(validate_sprite_name("my-sprite").is_ok());
        assert!(validate_sprite_name("test123").is_ok());
        assert!(validate_sprite_name("a").is_ok());
        assert!(validate_sprite_name("a-b-c").is_ok());
    }

    #[test]
    fn test_validate_sprite_name_empty() {
        assert!(validate_sprite_name("").is_err());
    }

    #[test]
    fn test_validate_sprite_name_too_long() {
        let long_name = "a".repeat(64);
        assert!(validate_sprite_name(&long_name).is_err());
    }

    #[test]
    fn test_validate_sprite_name_invalid_chars() {
        assert!(validate_sprite_name("my sprite").is_err());
        assert!(validate_sprite_name("my/sprite").is_err());
        assert!(validate_sprite_name("MY-SPRITE").is_err());
        assert!(validate_sprite_name("../../admin").is_err());
        assert!(validate_sprite_name("name?extra=1").is_err());
    }

    #[test]
    fn test_validate_sprite_name_leading_trailing_hyphen() {
        assert!(validate_sprite_name("-bad").is_err());
        assert!(validate_sprite_name("bad-").is_err());
    }
}
