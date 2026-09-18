//! MCP server configurations contributed by capabilities.

use crate::mcp_server::{McpServerActsAs, ScopedMcpServer, ScopedMcpServers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MCP server contributed by a capability.
///
/// Unlike an explicit scoped attachment, a contribution must declare `actsAs`.
/// The wrapper makes omission unrepresentable for native capabilities and
/// rejects serialized declarative or plugin contributions that omit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMcpServer(ScopedMcpServer);

impl CapabilityMcpServer {
    /// Build a capability contribution with an explicit acting identity.
    pub fn new(mut server: ScopedMcpServer, acts_as: McpServerActsAs) -> Self {
        server.acts_as = acts_as;
        Self(server)
    }

    /// Borrow the contribution as its runtime scoped-server representation.
    pub fn as_scoped(&self) -> &ScopedMcpServer {
        &self.0
    }

    /// Mutably borrow the contribution as its scoped-server representation.
    pub fn as_scoped_mut(&mut self) -> &mut ScopedMcpServer {
        &mut self.0
    }

    /// Consume the contribution and return its scoped-server representation.
    pub fn into_scoped(self) -> ScopedMcpServer {
        self.0
    }
}

impl std::ops::Deref for CapabilityMcpServer {
    type Target = ScopedMcpServer;

    fn deref(&self) -> &Self::Target {
        self.as_scoped()
    }
}

impl std::ops::DerefMut for CapabilityMcpServer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_scoped_mut()
    }
}

impl Serialize for CapabilityMcpServer {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut value =
            serde_json::to_value(self.as_scoped()).map_err(serde::ser::Error::custom)?;
        let object = value.as_object_mut().ok_or_else(|| {
            serde::ser::Error::custom("capability MCP server must serialize as an object")
        })?;
        object.insert(
            "actsAs".to_string(),
            serde_json::to_value(self.acts_as).map_err(serde::ser::Error::custom)?,
        );
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CapabilityMcpServer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("capability MCP server must be an object"))?;
        if !object.contains_key("actsAs") && !object.contains_key("acts_as") {
            return Err(serde::de::Error::custom(
                "capability MCP server must declare actsAs",
            ));
        }
        let server = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self(server))
    }
}

/// MCP server contributions keyed by their capability-local names.
pub type CapabilityMcpServers = BTreeMap<String, CapabilityMcpServer>;

/// Convert capability contributions at the runtime scoped-server merge seam.
pub fn capability_mcp_servers_to_scoped(servers: &CapabilityMcpServers) -> ScopedMcpServers {
    servers
        .iter()
        .map(|(name, server)| (name.clone(), server.as_scoped().clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn capability_config_requires_and_always_emits_acts_as() {
        assert!(
            serde_json::from_value::<CapabilityMcpServer>(json!({
                "url": "https://example.com/mcp"
            }))
            .unwrap_err()
            .to_string()
            .contains("must declare actsAs")
        );

        for (wire_name, acts_as) in [
            ("none", McpServerActsAs::None),
            ("service", McpServerActsAs::Service),
            ("user", McpServerActsAs::User),
        ] {
            let config: CapabilityMcpServer = serde_json::from_value(json!({
                "url": "https://example.com/mcp",
                "actsAs": wire_name
            }))
            .unwrap();
            assert_eq!(config.acts_as, acts_as);
            assert_eq!(
                serde_json::to_value(config).unwrap(),
                json!({"type":"http","url":"https://example.com/mcp","actsAs":wire_name})
            );
        }
    }
}
