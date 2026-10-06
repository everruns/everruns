// User MCP servers capability (knowledge/integrations/user-mcp-servers.md, D2).
//
// With `use` on (the default), the MCP servers the chatting person added for
// themselves join the agent's MCP servers for that person's turns. The
// capability contributes no tools and no static servers: which servers apply
// depends on who sent the message, so the hosted control plane resolves them
// per turn from the person's own list.
//
// Decision: only the turn's verified initiating person counts, never the
// session owner or the agent's service account. Unattended runs (triggers,
// schedules) have no such person and get nothing, and sessions with several
// people get nothing, so one person's tools never appear in another's turn.
// An agent or capability server with the same name wins over a user server.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use serde_json::{Value, json};

pub const USER_MCP_CAPABILITY_ID: &str = "user_mcp";

/// Whether the person's own MCP servers join their turns (`use`, default on).
pub fn user_mcp_use_enabled(config: &Value) -> bool {
    config.get("use").and_then(Value::as_bool).unwrap_or(true)
}

/// Lets an agent use the MCP servers the chatting person added for themselves.
pub struct UserMcpCapability;

impl Capability for UserMcpCapability {
    fn id(&self) -> &str {
        USER_MCP_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "User MCP Servers"
    }

    fn description(&self) -> &str {
        "Use the MCP servers the person chatting added for themselves in Settings, signed in as that person."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "MCP-сервери користувача",
            "Використовуйте MCP-сервери, які людина в розмові додала для себе в Налаштуваннях, від імені цієї людини.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("plug")
    }

    fn category(&self) -> Option<&str> {
        Some("Integrations")
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "use": {
                    "type": "boolean",
                    "title": "Use the person's MCP servers",
                    "description": "Add the enabled MCP servers of the person chatting to their turns.",
                    "default": true
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let Some(object) = config.as_object() else {
            return Err("user_mcp config must be an object".to_string());
        };
        for (key, value) in object {
            match key.as_str() {
                "use" if value.is_boolean() => {}
                "use" => return Err("user_mcp.use must be a boolean".to_string()),
                other => return Err(format!("Unknown user_mcp setting: {other}")),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_defaults_on_and_validates() {
        assert!(user_mcp_use_enabled(&json!({})));
        assert!(user_mcp_use_enabled(&Value::Null));
        assert!(!user_mcp_use_enabled(&json!({"use": false})));
        assert!(UserMcpCapability.validate_config(&json!({"use": true})).is_ok());
        assert!(UserMcpCapability.validate_config(&json!({"use": "yes"})).is_err());
        assert!(UserMcpCapability.validate_config(&json!({"manage": true})).is_err());
    }
}
