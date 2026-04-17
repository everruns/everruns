// Capability commands — user-facing read-only operations.
//
// Capabilities are a bounded registry (~30-50 items). No policy checks
// needed — these are public read endpoints.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::CapabilityInfo;
use crate::domains::common::*;
use everruns_core::CapabilityId;
use serde::Deserialize;

// Capabilities are a bounded set (~30-50 items), so default to showing all.
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 200;

// ============================================================================
// ListCapabilities
// ============================================================================

/// List available capabilities with optional search and pagination.
#[derive(Debug, Deserialize)]
pub struct ListCapabilities {
    pub search: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

impl Command for ListCapabilities {
    type Output = Paginated<CapabilityInfo>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_capabilities",
            category: "capabilities",
            description: "List available capabilities. Use search for name/description filtering. Supports pagination (limit/offset).",
            method: "GET",
            path: "/v1/capabilities",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<Paginated<CapabilityInfo>, CommandError> {
        let mut capabilities = ctx
            .capability_service
            .list_all(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;

        if let Some(ref search) = self.search {
            q::filter_by_search(&mut capabilities, search);
        }

        let total = capabilities.len() as u32;
        let offset = self.offset.unwrap_or(0);
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

        let data: Vec<CapabilityInfo> = capabilities
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();

        Ok(Paginated {
            data,
            total,
            offset,
            limit,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListCapabilities>() }

// ============================================================================
// GetCapability
// ============================================================================

/// Get a specific capability by ID.
#[derive(Debug, Deserialize)]
pub struct GetCapability {
    pub id: String,
}

impl Command for GetCapability {
    type Output = CapabilityInfo;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_capability",
            category: "capabilities",
            description: "Get a specific capability by ID.",
            method: "GET",
            path: "/v1/capabilities/{id}",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<CapabilityInfo, CommandError> {
        let cap_id = CapabilityId::new(&self.id);

        ctx.capability_service
            .get(ctx.org_id(), &cap_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Capability"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetCapability>() }
