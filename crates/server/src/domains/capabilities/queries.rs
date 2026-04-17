// Capability query helpers.
//
// Capabilities are a read-only registry backed by CapabilityService.
// No direct DB access — all reads delegate to the service which combines
// built-in capabilities, MCP servers, and skills.
//
// Filtering and pagination are done in-memory since the registry is small
// (~30-50 items).

use super::types::CapabilityInfo;

/// Filter capabilities by search query (name/description match).
pub fn filter_by_search(capabilities: &mut Vec<CapabilityInfo>, search: &str) {
    capabilities.retain(|c| c.matches_search(search));
}
