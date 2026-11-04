//! Desktop of the Session's primary Sandbox.
//!
//! A Sandbox Template with the desktop option (`target.options.desktop`) makes
//! the provider create the primary Sandbox with a desktop, and the server adds
//! this capability next to `session_sandbox` with the same config. The
//! `computer` tool then drives that Sandbox's display: the same machine, files
//! and network rules the shell and file tools use.
//!
//! Decision: a capability of its own rather than a branch inside
//! `session_sandbox`, because the computer-use system prompt is static per
//! capability and must only reach agents that have a desktop.

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityStatus, RiskLevel};
use everruns_core::computer_use::{
    COMPUTER_TOOL_NAME, COMPUTER_USE_SYSTEM_PROMPT, ComputerBackend, ComputerSession, ComputerTool,
    ComputerUseConfig, DisplaySize,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::Value;

use crate::sandbox_state::load_state_for_provider;
use crate::session_sandbox::{
    DesktopRecording, SessionSandboxConfig, SessionSandboxStatus, create_session_sandbox_provider,
    ensure_session_sandbox_running,
};

pub const SANDBOX_DESKTOP_CAPABILITY_ID: &str = "sandbox_desktop";

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{COMPUTER_USE_SYSTEM_PROMPT} The display is the desktop of the primary Sandbox, a Linux \
         desktop rather than a browser page: there is no navigate action, so open applications \
         from the desktop and type addresses into them. Files you write with the shell and file \
         tools are on the same machine."
    )
});

pub struct SandboxDesktopCapability;

impl Capability for SandboxDesktopCapability {
    fn localizations(&self) -> Vec<everruns_core::capabilities::CapabilityLocalization> {
        vec![everruns_core::capabilities::CapabilityLocalization::text(
            "uk",
            "Робочий стіл пісочниці",
            "Дозволяє агенту працювати з робочим столом основної пісочниці сесії: робити знімки екрана, натискати, вводити текст, прокручувати та використовувати клавіші.",
        )]
    }

    fn id(&self) -> &str {
        SANDBOX_DESKTOP_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Sandbox Desktop"
    }

    fn description(&self) -> &str {
        "Let the agent see and operate the desktop of the Session's primary Sandbox: screenshots, \
         clicks, typing, scrolling and keys at screen coordinates, in any desktop app. Added by a \
         Sandbox Template with the desktop option."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("monitor")
    }

    fn category(&self) -> Option<&str> {
        Some("Sandboxes")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        let sandbox = serde_json::from_value::<SessionSandboxConfig>(config.clone()).ok();
        vec![Box::new(ComputerTool::new(
            Arc::new(SandboxDesktopBackend { sandbox }),
            ComputerUseConfig::from_value_or_default(&desktop_settings(config)),
        ))]
    }

    fn driver_options(&self, config: &Value) -> Vec<(String, Value)> {
        // Native OpenAI / Anthropic computer tools where the model has one,
        // sized to the Sandbox display.
        ComputerUseConfig::from_value_or_default(&desktop_settings(config)).driver_options()
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn exclusive_group(&self) -> Option<&'static str> {
        // One display per agent, like every other computer-use backend.
        Some(COMPUTER_TOOL_NAME)
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        let sandbox: SessionSandboxConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid sandbox_desktop config: {e}"))?;
        let Some(settings) = sandbox.desktop() else {
            return Err("sandbox_desktop needs a Sandbox created with the desktop option".into());
        };
        ComputerUseConfig::from_value(&settings)
            .map(|_| ())
            .map_err(|e| format!("invalid desktop settings: {e}"))
    }
}

/// The desktop settings (`provider_config.desktop`) of a sandbox config, or
/// null for defaults.
fn desktop_settings(config: &Value) -> Value {
    serde_json::from_value::<SessionSandboxConfig>(config.clone())
        .ok()
        .and_then(|sandbox| sandbox.desktop())
        .unwrap_or(Value::Null)
}

/// The `computer` backend on the primary Sandbox's desktop.
struct SandboxDesktopBackend {
    sandbox: Option<SessionSandboxConfig>,
}

#[async_trait]
impl ComputerBackend for SandboxDesktopBackend {
    fn id(&self) -> &str {
        "sandbox_desktop"
    }

    fn supports_navigation(&self) -> bool {
        false
    }

    async fn acquire(
        &self,
        context: &ToolContext,
        display: DisplaySize,
    ) -> Result<Box<dyn ComputerSession>, ToolExecutionResult> {
        let config = self.sandbox.as_ref().ok_or_else(|| {
            ToolExecutionResult::tool_error("the Sandbox desktop is not configured")
        })?;
        let provider = create_session_sandbox_provider(&config.provider).ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Session sandbox provider '{}' is not registered",
                config.provider
            ))
        })?;
        let state = ensure_session_sandbox_running(context, config).await?;
        provider
            .open_desktop(context, config, &state.instance, display)
            .await
    }
}

/// Stop and take the recordings of the Session's desktop, when it has a
/// running Sandbox created with a recorded desktop. The server calls this when
/// the Session goes idle; it never starts or resumes a Sandbox.
pub async fn collect_desktop_recordings(
    context: &ToolContext,
    config: &SessionSandboxConfig,
) -> Result<Vec<DesktopRecording>, ToolExecutionResult> {
    if !config.desktop_recording() {
        return Ok(Vec::new());
    }
    let Some(state) = load_state_for_provider(context, &config.provider).await? else {
        return Ok(Vec::new());
    };
    if state.status != SessionSandboxStatus::Running {
        return Ok(Vec::new());
    }
    let Some(provider) = create_session_sandbox_provider(&config.provider) else {
        return Ok(Vec::new());
    };
    provider
        .collect_desktop_recordings(context, config, &state.instance)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requires_a_sandbox_created_with_a_desktop() {
        let capability = SandboxDesktopCapability;
        let with_desktop = json!({
            "provider": "daytona",
            "provider_config": {"desktop": {"display_width": 1280, "display_height": 800}}
        });
        assert!(capability.validate_config(&with_desktop).is_ok());
        assert!(
            capability
                .validate_config(
                    &json!({"provider": "daytona", "provider_config": {"desktop": true}})
                )
                .is_ok()
        );
        let without = json!({"provider": "daytona", "provider_config": {}});
        assert!(capability.validate_config(&without).is_err());
        let bad = json!({"provider": "daytona", "provider_config": {"desktop": {"nope": 1}}});
        assert!(capability.validate_config(&bad).is_err());
    }

    #[test]
    fn offers_the_computer_tool() {
        let tools = SandboxDesktopCapability.tools_with_config(&json!({
            "provider": "e2b",
            "provider_config": {"desktop": true}
        }));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "computer");
    }

    #[test]
    fn native_tool_options_follow_the_desktop_settings() {
        let sized = SandboxDesktopCapability.driver_options(&json!({
            "provider": "e2b",
            "provider_config": {"desktop": {"display_width": 1024, "display_height": 768}}
        }));
        assert_eq!(
            sized,
            ComputerUseConfig::from_value_or_default(
                &json!({"display_width": 1024, "display_height": 768})
            )
            .driver_options()
        );
        let off = SandboxDesktopCapability.driver_options(&json!({
            "provider": "e2b",
            "provider_config": {"desktop": {"native_tools": false}}
        }));
        assert!(off.is_empty(), "{off:?}");
    }
}
