//! Shared bounds and validation for complete MCP tool discovery.

use crate::McpToolDefinition;
use anyhow::Result;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Maximum number of tools accepted from one MCP server.
pub const MCP_DISCOVERY_MAX_TOOLS: usize = 10_000;
/// Maximum serialized size of one complete MCP tool catalog.
pub const MCP_DISCOVERY_MAX_BYTES: usize = 16 * 1024 * 1024;
/// Maximum MCP tools attached to one turn across all configured servers.
pub const MCP_SESSION_DISCOVERY_MAX_TOOLS: usize = 10_000;
/// Maximum serialized, server-prefixed MCP definitions attached to one turn.
pub const MCP_SESSION_DISCOVERY_MAX_BYTES: usize = 32 * 1024 * 1024;
/// Maximum number of pages accepted from one MCP server.
pub const MCP_DISCOVERY_MAX_PAGES: usize = 256;
/// Maximum wall time spent fetching one complete catalog.
pub const MCP_DISCOVERY_DEADLINE: Duration = Duration::from_secs(60);

/// Discovery failed before a complete and unambiguous catalog was available.
#[derive(Debug, thiserror::Error)]
pub enum McpDiscoveryError {
    #[error("MCP tool discovery incomplete: exceeded {limit} {resource}")]
    LimitExceeded {
        resource: &'static str,
        limit: usize,
    },
    #[error("MCP tool discovery incomplete: exceeded {0:?} deadline")]
    DeadlineExceeded(Duration),
    #[error("MCP tool discovery incomplete: duplicate tool name '{0}'")]
    DuplicateToolName(String),
    #[error("MCP tool discovery incomplete: server returned an empty cursor")]
    EmptyCursor,
    #[error("MCP tool discovery incomplete: server repeated a pagination cursor")]
    RepeatedCursor,
}

pub(crate) fn check_page_bytes(bytes: usize) -> Result<()> {
    if bytes > MCP_DISCOVERY_MAX_BYTES {
        return Err(McpDiscoveryError::LimitExceeded {
            resource: "page bytes",
            limit: MCP_DISCOVERY_MAX_BYTES,
        }
        .into());
    }
    Ok(())
}

// THREAT[TM-DOS-031]: Bound complete discovery before admitting any catalog.
// THREAT[TM-TOOL-002]: Reject ambiguous or incomplete server-controlled pages.
/// Accumulates pages while enforcing complete, unambiguous discovery.
pub(crate) struct DiscoveryAccumulator {
    started: Instant,
    pages: usize,
    bytes: usize,
    response_bytes: usize,
    tools: Vec<McpToolDefinition>,
    names: HashSet<String>,
    cursors: HashSet<String>,
}

impl DiscoveryAccumulator {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            pages: 0,
            bytes: 0,
            response_bytes: 0,
            tools: Vec::new(),
            names: HashSet::new(),
            cursors: HashSet::new(),
        }
    }

    /// Add a page and return its continuation cursor. Any violation is an
    /// error: callers must never turn an incomplete catalog into a success.
    pub(crate) fn push_page(
        &mut self,
        tools: Vec<McpToolDefinition>,
        next_cursor: Option<String>,
    ) -> Result<Option<String>> {
        self.pages += 1;
        if self.pages > MCP_DISCOVERY_MAX_PAGES {
            return Err(McpDiscoveryError::LimitExceeded {
                resource: "pages",
                limit: MCP_DISCOVERY_MAX_PAGES,
            }
            .into());
        }
        if self.started.elapsed() > MCP_DISCOVERY_DEADLINE {
            return Err(McpDiscoveryError::DeadlineExceeded(MCP_DISCOVERY_DEADLINE).into());
        }
        for tool in tools {
            if !self.names.insert(tool.name.clone()) {
                return Err(McpDiscoveryError::DuplicateToolName(tool.name).into());
            }
            self.bytes = self.bytes.saturating_add(serde_json::to_vec(&tool)?.len());
            if self.bytes > MCP_DISCOVERY_MAX_BYTES {
                return Err(McpDiscoveryError::LimitExceeded {
                    resource: "serialized bytes",
                    limit: MCP_DISCOVERY_MAX_BYTES,
                }
                .into());
            }
            self.tools.push(tool);
            if self.tools.len() > MCP_DISCOVERY_MAX_TOOLS {
                return Err(McpDiscoveryError::LimitExceeded {
                    resource: "tools",
                    limit: MCP_DISCOVERY_MAX_TOOLS,
                }
                .into());
            }
        }
        if let Some(cursor) = next_cursor.as_deref() {
            if cursor.is_empty() {
                return Err(McpDiscoveryError::EmptyCursor.into());
            }
            if !self.cursors.insert(cursor.to_owned()) {
                return Err(McpDiscoveryError::RepeatedCursor.into());
            }
        }
        Ok(next_cursor)
    }

    pub(crate) fn add_response_bytes(&mut self, bytes: usize) -> Result<()> {
        self.response_bytes = self.response_bytes.saturating_add(bytes);
        if self.response_bytes > MCP_DISCOVERY_MAX_BYTES {
            return Err(McpDiscoveryError::LimitExceeded {
                resource: "discovery response bytes",
                limit: MCP_DISCOVERY_MAX_BYTES,
            }
            .into());
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Vec<McpToolDefinition> {
        self.tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: impl Into<String>) -> McpToolDefinition {
        McpToolDefinition {
            name: name.into(),
            title: None,
            description: None,
            input_schema: json!({"type":"object"}),
            annotations: None,
        }
    }

    #[test]
    fn accepts_the_ten_thousand_tool_limit() {
        let mut discovery = DiscoveryAccumulator::new();
        for page in 0..100 {
            let tools = (page * 100..(page + 1) * 100)
                .map(|i| tool(format!("tool_{i}")))
                .collect();
            let cursor = (page < 99).then(|| format!("page_{}", page + 1));
            discovery.push_page(tools, cursor).unwrap();
        }
        assert_eq!(discovery.finish().len(), MCP_DISCOVERY_MAX_TOOLS);
    }

    #[test]
    fn rejects_catalog_over_byte_limit() {
        let mut discovery = DiscoveryAccumulator::new();
        let huge = McpToolDefinition {
            name: "large_schema".into(),
            title: None,
            description: Some("x".repeat(MCP_DISCOVERY_MAX_BYTES)),
            input_schema: json!({"type":"object"}),
            annotations: None,
        };
        assert!(discovery.push_page(vec![huge], None).is_err());
    }

    #[test]
    fn page_byte_bound_accepts_exact_limit_and_rejects_limit_plus_one() {
        assert!(check_page_bytes(MCP_DISCOVERY_MAX_BYTES).is_ok());
        let error = check_page_bytes(MCP_DISCOVERY_MAX_BYTES + 1).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<McpDiscoveryError>(),
            Some(McpDiscoveryError::LimitExceeded {
                resource: "page bytes",
                ..
            })
        ));

        let mut discovery = DiscoveryAccumulator::new();
        assert!(
            discovery
                .add_response_bytes(MCP_DISCOVERY_MAX_BYTES)
                .is_ok()
        );
        let error = discovery.add_response_bytes(1).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<McpDiscoveryError>(),
            Some(McpDiscoveryError::LimitExceeded {
                resource: "discovery response bytes",
                ..
            })
        ));
    }

    #[test]
    fn rejects_duplicate_names_and_repeated_cursors() {
        let mut duplicate = DiscoveryAccumulator::new();
        duplicate
            .push_page(vec![tool("same")], Some("next".into()))
            .unwrap();
        assert!(duplicate.push_page(vec![tool("same")], None).is_err());

        let mut repeated = DiscoveryAccumulator::new();
        repeated
            .push_page(vec![tool("one")], Some("next".into()))
            .unwrap();
        assert!(
            repeated
                .push_page(vec![tool("two")], Some("next".into()))
                .is_err()
        );

        let mut empty = DiscoveryAccumulator::new();
        assert!(
            empty
                .push_page(vec![tool("empty")], Some(String::new()))
                .is_err()
        );
    }

    #[test]
    fn rejects_catalog_over_tool_limit_without_truncating() {
        let mut discovery = DiscoveryAccumulator::new();
        let tools = (0..=MCP_DISCOVERY_MAX_TOOLS)
            .map(|i| tool(format!("tool_{i}")))
            .collect();
        assert!(discovery.push_page(tools, None).is_err());
    }

    #[test]
    fn rejects_catalog_over_page_limit_without_truncating() {
        let mut discovery = DiscoveryAccumulator::new();
        for page in 0..MCP_DISCOVERY_MAX_PAGES {
            discovery
                .push_page(
                    vec![tool(format!("tool_{page}"))],
                    Some(format!("cursor_{page}")),
                )
                .unwrap();
        }
        assert!(discovery.push_page(vec![tool("last")], None).is_err());
    }
}
