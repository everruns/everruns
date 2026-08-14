//! Persistent Sprites microVM sandboxes for Everruns agents.
//!
//! This integration contributes tools for creating, managing, executing in, and
//! checkpointing Sprites to the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_integrations_sprites::SpritesCapability;
//!
//! let capability = SpritesCapability;
//! # let _ = capability;
//! ```

pub mod client;
pub mod connection;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_core::tools::Tool;
use everruns_platform::connector::ConnectorPlugin;

use std::sync::LazyLock;

use connection::SpritesConnector;

use tools::{
    SpritesCheckpointTool, SpritesCreateSpriteTool, SpritesExecTool, SpritesListSpritesTool,
    SpritesManageSpriteTool, SpritesReadFileTool, SpritesRestoreCheckpointTool,
    SpritesServiceUrlTool, SpritesWriteFileTool,
};

// ============================================================================
// Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(SpritesCapability),
    }
}

inventory::submit! {
    ConnectorPlugin {
        experimental_only: true,
        factory: || Box::new(SpritesConnector),
    }
}

// ============================================================================
// Constants
// ============================================================================

const SPRITES_API_BASE: &str = "https://api.sprites.dev/v1";
const SPRITES_SECRET_PREFIX: &str = "sprites_sprite:";
const EXEC_TIMEOUT_MS: u64 = 120_000;
/// Default workspace path inside sprites
const SPRITES_WORKSPACE_PATH: &str = "/home/sprite";

// ============================================================================
// SpritesCapability
// ============================================================================

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        "Sprites are persistent Firecracker Linux VMs. Create or select a sprite before sprite-scoped operations; data survives idle/sleep, checkpoints can protect risky changes, services should listen on port 8080 for the public URL, and deleting avoids storage charges. Working directory is `/home/sprite`.",
    );
    prompt.push_str(everruns_core::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

pub struct SpritesCapability;

impl Capability for SpritesCapability {
    fn id(&self) -> &str {
        "sprites"
    }

    fn name(&self) -> &str {
        "Sprites"
    }

    fn description(&self) -> &str {
        "Run code in persistent, hardware-isolated Linux microVMs powered by Sprites. \
         Create multiple Firecracker VMs per session with full ext4 filesystems, \
         execute commands, manage files, checkpoint/restore state, and expose HTTP services."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Sprites",
            "Запускайте код у постійних, апаратно ізольованих мікровіртуальних машинах Linux \
             на основі Sprites. Створюйте кілька віртуальних машин Firecracker на сесію з \
             повноцінними файловими системами ext4, виконуйте команди, керуйте файлами, \
             зберігайте та відновлюйте стан через контрольні точки й публікуйте HTTP-сервіси.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("sprites")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SpritesCreateSpriteTool),
            Box::new(SpritesExecTool),
            Box::new(SpritesReadFileTool),
            Box::new(SpritesWriteFileTool),
            Box::new(SpritesListSpritesTool),
            Box::new(SpritesManageSpriteTool),
            Box::new(SpritesCheckpointTool),
            Box::new(SpritesRestoreCheckpointTool),
            Box::new(SpritesServiceUrlTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::CapabilityStatus;

    #[test]
    fn test_capability_metadata() {
        let cap = SpritesCapability;
        assert_eq!(cap.id(), "sprites");
        assert_eq!(cap.name(), "Sprites");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("sprites"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = SpritesCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 9);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"sprites_create_sprite"));
        assert!(names.contains(&"sprites_exec"));
        assert!(names.contains(&"sprites_read_file"));
        assert!(names.contains(&"sprites_write_file"));
        assert!(names.contains(&"sprites_list_sprites"));
        assert!(names.contains(&"sprites_manage_sprite"));
        assert!(names.contains(&"sprites_checkpoint"));
        assert!(names.contains(&"sprites_restore_checkpoint"));
        assert!(names.contains(&"sprites_service_url"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SpritesCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("Sprites"));
        assert!(prompt.contains("persistent Firecracker"));
        assert!(prompt.contains("checkpoints"));
        assert!(prompt.contains("port 8080"));
        assert!(prompt.contains("/home/sprite"));
    }

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = SpritesCapability;
        let ctx = everruns_core::capabilities::SystemPromptContext::without_file_store(
            everruns_provider::typed_id::SessionId::new(),
        );
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        // Bumped 1200 → 1400: EVE-778 grew the shared EXEC_OUTPUT_HINT with the
        // single-read/contextual-search policy (+438 bytes), taking this
        // contribution to 1361 bytes.
        assert!(prompt.len() <= 1400, "prompt is {} bytes", prompt.len());
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = SpritesCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "Tool {} should require context",
                tool.name()
            );
        }
    }

    #[test]
    fn test_capability_dependencies() {
        let cap = SpritesCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }
}
