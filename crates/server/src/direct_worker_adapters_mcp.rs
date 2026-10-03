//! MCP server descriptor mapping for `DirectWorkerAdapters`.
//!
//! Split out of `super::direct_worker_adapters` to keep that file under the
//! source-size ratchet.

use crate::domains::mcp_servers::McpServerResolved;
use everruns_worker::mcp_executor::McpServerInfo;
use std::collections::HashMap;

pub(crate) fn resolved_mcp_server_to_worker_info(
    resolved: McpServerResolved,
    secret_bindings: HashMap<String, Vec<everruns_core::mcp::McpSecretBinding>>,
) -> McpServerInfo {
    McpServerInfo {
        id: resolved.id,
        name: resolved.name,
        url: resolved.url,
        api_key: resolved.api_key,
        headers: resolved.headers,
        auth_mode: resolved.auth_mode,
        protocol_mode: resolved.protocol_mode,
        elicitation_policy: resolved.elicitation_policy,
        oauth_provider_id: resolved.oauth_provider_id,
        acts_as: resolved.acts_as,
        secret_bindings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn direct_mcp_adapter_preserves_neutral_catalog_descriptors() {
        for acts_as in [
            everruns_core::McpServerActsAs::None,
            everruns_core::McpServerActsAs::Service,
            everruns_core::McpServerActsAs::User,
        ] {
            let resolved = McpServerResolved {
                id: Uuid::new_v4(),
                name: "linear".to_string(),
                url: "https://mcp.linear.app/mcp".to_string(),
                auth_mode: everruns_core::McpServerAuthMode::None,
                protocol_mode: everruns_core::McpProtocolMode::Auto,
                elicitation_policy: Default::default(),
                oauth_provider_id: None,
                acts_as,
                api_key: None,
                headers: HashMap::new(),
            };

            let info = resolved_mcp_server_to_worker_info(resolved, HashMap::new());

            assert_eq!(info.acts_as, acts_as);
            assert_eq!(info.auth_mode, everruns_core::McpServerAuthMode::None);
            assert!(info.oauth_provider_id.is_none());
            assert!(info.api_key.is_none());
        }
    }
}
