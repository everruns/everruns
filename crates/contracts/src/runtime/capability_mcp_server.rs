//! MCP server configurations contributed by capabilities.

use crate::runtime::mcp_server::{
    McpCapabilityContext, McpServerActsAs, ScopedMcpServer, ScopedMcpServers,
};
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

/// Clone contributions and derive their owner context from the current capability.
/// Authored server descriptions and transport configuration stay unchanged.
pub fn capability_mcp_servers_to_scoped_with_context(
    servers: &CapabilityMcpServers,
    id: &str,
    name: &str,
    description: &str,
) -> ScopedMcpServers {
    let context = McpCapabilityContext {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
    };
    servers
        .iter()
        .map(|(name, server)| {
            let mut server = server.as_scoped().clone();
            server.capability_context = Some(context.clone());
            (name.clone(), server)
        })
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

    #[test]
    fn runtime_owner_context_is_cloned_rederived_and_absent_from_authored_wire() {
        let source: CapabilityMcpServer = serde_json::from_value(json!({
            "url":"https://example.com/mcp", "actsAs":"none",
            "description":"Server context", "headers":{"X-Route":"inventory"},
            "capability_context":{"id":"forged", "name":"Forged", "description":"secret"}
        }))
        .unwrap();
        assert!(source.capability_context.is_none());
        let authored = serde_json::to_value(source.as_scoped()).unwrap();
        let servers = CapabilityMcpServers::from([("gateway".into(), source)]);
        let scoped = capability_mcp_servers_to_scoped_with_context(
            &servers,
            "owner",
            "Inventory",
            "Cobalt warehouse",
        );
        let context = scoped["gateway"].capability_context.as_ref().unwrap();
        assert_eq!(context.id, "owner");
        assert_eq!(context.name, "Inventory");
        assert_eq!(context.description, "Cobalt warehouse");
        assert_eq!(
            scoped["gateway"].clone().capability_context.as_ref(),
            Some(context)
        );
        assert_eq!(
            scoped["gateway"].description.as_deref(),
            Some("Server context")
        );
        assert!(servers["gateway"].capability_context.is_none());
        assert_eq!(serde_json::to_value(&scoped["gateway"]).unwrap(), authored);
        let reloaded: ScopedMcpServer = serde_json::from_value(authored).unwrap();
        assert!(reloaded.capability_context.is_none());
        let stale = CapabilityMcpServers::from([(
            "gateway".into(),
            CapabilityMcpServer::new(scoped["gateway"].clone(), McpServerActsAs::None),
        )]);
        let updated = capability_mcp_servers_to_scoped_with_context(
            &stale,
            "replacement",
            "Replacement",
            "Updated metadata",
        );
        assert_eq!(
            updated["gateway"].capability_context.as_ref().unwrap().id,
            "replacement"
        );
        assert_eq!(
            updated["gateway"].description,
            scoped["gateway"].description
        );
    }
}
