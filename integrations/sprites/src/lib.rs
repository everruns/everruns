//! Sprites Integration
//!
//! Persistent, hardware-isolated Linux microVMs via Sprites (Fly.io) REST API.
//! Sprites are Firecracker VMs with full ext4 filesystems that persist across sessions.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: API token resolved via user connections (Settings > Connections), not secrets store
//! Decision: Per-sprite state stored in session secrets (encrypted at rest)
//! Decision: Single API tier — all operations go through api.sprites.dev/v1
//! Decision: Embrace persistence — sprites survive idle, expose HTTP endpoints, support checkpoints

pub mod client;
pub mod connection;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::tools::Tool;

use connection::SpritesConnectionProvider;

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
        factory: || Box::new(SpritesCapability),
    }
}

inventory::submit! {
    ConnectionProviderPlugin {
        experimental_only: true,
        factory: || Box::new(SpritesConnectionProvider),
    }
}

// ============================================================================
// Constants
// ============================================================================

const SPRITES_API_BASE: &str = "https://api.sprites.dev/v1";
const SPRITES_SECRET_PREFIX: &str = "sprites_sprite:";
const EXEC_TIMEOUT_MS: u64 = 120_000;
/// Default workspace path inside sprites
const SPRITES_WORKSPACE_PATH: &str = "/home/user";

// ============================================================================
// SpritesCapability
// ============================================================================

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
        Some(
            r#"## Sprites

Persistent, hardware-isolated Linux microVMs (Firecracker) via Sprites. Each sprite is
a full Linux computer with a persistent ext4 filesystem that survives between sessions.

Authentication: Sprites API token is resolved automatically from Settings > Connections > Sprites.
If not configured, guide the user to set up their token in Settings > Connections.

All tools except `sprites_create_sprite` and `sprites_list_sprites` require a `sprite_name`.
Sprites persist indefinitely — they hibernate when idle (no compute charges) and wake instantly.
Always DELETE sprites when done to avoid storage charges.

Key differences from ephemeral sandboxes:
- **Persistent filesystem**: Data survives idle/sleep, backed to durable object storage.
- **Checkpoints**: Use `sprites_checkpoint` to snapshot state before risky operations,
  `sprites_restore_checkpoint` to roll back. Checkpoints complete in ~300ms.
- **HTTP services**: Each sprite has a unique public URL. Listen on port 8080 inside
  the sprite and it's accessible at the sprite's URL. Use `sprites_service_url` to get
  the URL for sharing or testing.
- **Instant wake**: Sprites wake from hibernation in <1s when you execute a command.

Working directory is `/home/user`."#,
        )
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
        assert!(prompt.contains("Settings > Connections"));
        assert!(prompt.contains("sprites_create_sprite"));
        assert!(prompt.contains("sprites_checkpoint"));
        assert!(prompt.contains("HTTP services"));
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
