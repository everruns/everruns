//! Discovery: `initialize` (2025 protocols) and `server/discover` (MCP
//! 2026-07-28), and the capability and extension maps they advertise.

use serde_json::{Value, json};

use super::{
    InitializeParams, JsonRpcResponse, MCP_PROTOCOL_VERSION_LATEST, MCP_SERVER_NAME,
    MCP_SERVER_VERSION, SUPPORTED_PROTOCOL_VERSIONS, apps, negotiate_protocol_version, tasks,
};

pub(super) fn handle_initialize(id: Option<Value>, params: Value) -> JsonRpcResponse {
    let params: InitializeParams = serde_json::from_value(params).unwrap_or_default();
    let protocol_version = negotiate_protocol_version(params.protocol_version.as_deref());
    let mut capabilities = json!({
        "tools": {
            "listChanged": false
        },
        "resources": {}
    });
    // Advertise the Tasks extension (SEP-2663) only under the negotiated
    // 2026-07-28 protocol. 2025-* clients see the capabilities shape unchanged.
    if let Some(extensions) = server_extensions(protocol_version) {
        capabilities["extensions"] = extensions;
    }
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": protocol_version,
            "capabilities": capabilities,
            "serverInfo": {
                "name": MCP_SERVER_NAME,
                "version": MCP_SERVER_VERSION
            }
        }),
    )
}

/// Extensions advertised under the 2026-07-28 protocol: Tasks (SEP-2663) and
/// MCP Apps (SEP-1865). `None` for 2025-* so their `initialize` shape is
/// unchanged.
fn server_extensions(protocol_version: &str) -> Option<Value> {
    let mut extensions = tasks::initialize_extensions(protocol_version)?;
    extensions[apps::UI_EXTENSION_KEY] = apps::server_extension();
    Some(extensions)
}

/// `server/discover` (MCP 2026-07-28): versions, capabilities and server info
/// in one stateless call, the replacement for `initialize`.
/// `events` is advertised only where `events/*` answer: the org opted into
/// MCP Events.
pub(super) fn handle_server_discover(id: Option<Value>, events: bool) -> JsonRpcResponse {
    let mut capabilities = json!({ "tools": { "listChanged": false }, "resources": {} });
    if let Some(extensions) = server_extensions(MCP_PROTOCOL_VERSION_LATEST) {
        capabilities["extensions"] = extensions;
    }
    if events {
        capabilities["events"] = json!({});
    }
    JsonRpcResponse::success(
        id,
        json!({
            "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS,
            "capabilities": capabilities,
            "_meta": {
                "io.modelcontextprotocol/serverInfo": {
                    "name": MCP_SERVER_NAME,
                    "version": MCP_SERVER_VERSION
                }
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{MCP_SERVER_NAME, MCP_SERVER_VERSION};
    use super::{handle_initialize, handle_server_discover};
    use serde_json::json;

    #[test]
    fn initialize_reports_the_package_version() {
        let result = handle_initialize(Some(json!(1)), json!({}))
            .result
            .expect("result");
        assert_eq!(result["serverInfo"]["name"], MCP_SERVER_NAME);
        assert_eq!(result["serverInfo"]["version"], MCP_SERVER_VERSION);
        assert_eq!(MCP_SERVER_VERSION, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn server_discover_reports_the_package_version() {
        let result = handle_server_discover(Some(json!(1)), false)
            .result
            .expect("result");
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            MCP_SERVER_NAME
        );
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
            MCP_SERVER_VERSION
        );
    }
}
