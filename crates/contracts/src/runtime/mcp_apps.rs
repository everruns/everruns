// MCP Apps (SEP-1865) as a host: views that connected MCP servers ship.
//
// A server links a tool to an interactive view with `_meta.ui.resourceUri`
// (`ui://...`, MIME `text/html;profile=mcp-app`). When such a tool runs, the
// proxy tool records which view belongs to the call in the tool result's
// `details` under [`MCP_APP_DETAILS_KEY`]. The model never sees `details`;
// the console renders the view in a sandboxed frame and channels without a
// browser (Slack) post a rendered picture. The view's HTML is read from the
// server when someone looks at it, not stored with the event.
//
// See knowledge/integrations/mcp-apps-host.md.

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Extension key a client declares in its capabilities to say it renders views.
pub const MCP_UI_EXTENSION_KEY: &str = "io.modelcontextprotocol/ui";
/// The only MIME type the MCP Apps extension defines for views.
pub const MCP_APP_MIME_TYPE: &str = "text/html;profile=mcp-app";
/// Key in `ToolHints::metadata` and in `ToolResult::details` for a view.
pub const MCP_APP_DETAILS_KEY: &str = "mcp_app";
/// URI scheme every view uses.
const UI_SCHEME: &str = "ui://";
/// Longest view URI accepted, so a server cannot stuff payload into it.
const MAX_RESOURCE_URI_LEN: usize = 512;

/// The view a tool declares, read from its `_meta`.
///
/// Reads the nested `_meta.ui.resourceUri` and the deprecated flat
/// `_meta["ui/resourceUri"]`. Anything that is not a short `ui://` URI is
/// ignored: hosts must only load views the server serves itself.
pub fn tool_view_uri(meta: Option<&Value>) -> Option<&str> {
    let meta = meta?;
    let uri = meta
        .get("ui")
        .and_then(|ui| ui.get("resourceUri"))
        .or_else(|| meta.get("ui/resourceUri"))
        .and_then(Value::as_str)?;
    is_view_uri(uri).then_some(uri)
}

/// Whether `uri` is an acceptable view URI.
pub fn is_view_uri(uri: &str) -> bool {
    uri.len() > UI_SCHEME.len()
        && uri.len() <= MAX_RESOURCE_URI_LEN
        && uri.starts_with(UI_SCHEME)
        && !uri.chars().any(char::is_control)
}

/// Whether a tool is meant for its view only (`_meta.ui.visibility` without
/// `"model"`). Such tools are not offered to the model; a view may still call
/// them.
pub fn tool_is_app_only(meta: Option<&Value>) -> bool {
    let Some(visibility) = meta
        .and_then(|meta| meta.get("ui"))
        .and_then(|ui| ui.get("visibility"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    !visibility
        .iter()
        .any(|entry| entry.as_str() == Some("model"))
}

/// What a tool call's `details` records about its view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpAppCallDetails {
    /// Sanitized server prefix, as in the tool name `mcp_<server>__<tool>`.
    pub server: String,
    /// The server's own tool name.
    pub tool: String,
    /// The view to load (`ui://...`).
    pub resource_uri: String,
    /// The call's `structuredContent`, for the view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    /// The call result's `_meta`, for the view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl McpAppCallDetails {
    /// Read a view out of a tool result's `details`.
    pub fn from_details(details: Option<&Value>) -> Option<Self> {
        let value = details?.get(MCP_APP_DETAILS_KEY)?;
        let parsed: Self = serde_json::from_value(value.clone()).ok()?;
        is_view_uri(&parsed.resource_uri).then_some(parsed)
    }
}

/// Where an organization MCP server's views are shown. A display preference:
/// hosted runtimes always tell servers they can render views, so turning views
/// off never changes what a tool returns to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "on"))]
#[serde(rename_all = "snake_case")]
pub enum McpInteractiveViews {
    /// Views run in the console; Slack gets a picture of the view and a link.
    #[default]
    On,
    /// Views run in the console; Slack gets the text result only.
    ConsoleOnly,
    /// No views anywhere: every channel shows the text result.
    Off,
}

impl McpInteractiveViews {
    /// Whether this is the default. Keeps the field out of stored settings.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::On)
    }

    /// Whether the console should load the view.
    pub fn in_console(&self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Whether Slack should post a picture of the view.
    pub fn in_slack(&self) -> bool {
        matches!(self, Self::On)
    }
}

/// Key under which the MCP client keeps a result's `structuredContent` and
/// `_meta` in `ToolResult::details` until the proxy tool reads them.
pub const MCP_RESULT_DETAILS_KEY: &str = "mcp_result";

/// Security settings a view declares on its resource contents (`_meta.ui`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpAppViewPolicy {
    /// Origins the view may fetch from (`connect-src`).
    #[serde(default, rename = "connectDomains")]
    pub connect_domains: Vec<String>,
    /// Origins for scripts, styles, images, fonts and media.
    #[serde(default, rename = "resourceDomains")]
    pub resource_domains: Vec<String>,
    /// Origins for nested frames.
    #[serde(default, rename = "frameDomains")]
    pub frame_domains: Vec<String>,
    /// Whether the view asks for a visible border.
    #[serde(
        default,
        rename = "prefersBorder",
        skip_serializing_if = "Option::is_none"
    )]
    pub prefers_border: Option<bool>,
}

impl McpAppViewPolicy {
    /// Read the policy from a resource's `_meta`, keeping only well-formed
    /// `https://` origins (a wildcard subdomain is allowed). Everything else is
    /// dropped, so a server cannot widen the sandbox with `*` or `data:`.
    pub fn from_meta(meta: Option<&Value>) -> Self {
        let Some(ui) = meta.and_then(|meta| meta.get("ui")) else {
            return Self::default();
        };
        let csp = ui.get("csp");
        let origins = |key: &str| -> Vec<String> {
            csp.and_then(|csp| csp.get(key))
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|origin| is_allowed_origin(origin))
                        .take(32)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        Self {
            connect_domains: origins("connectDomains"),
            resource_domains: origins("resourceDomains"),
            frame_domains: origins("frameDomains"),
            prefers_border: ui.get("prefersBorder").and_then(Value::as_bool),
        }
    }
}

fn is_allowed_origin(origin: &str) -> bool {
    let Some(host) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("wss://"))
    else {
        return false;
    };
    let host = host.strip_prefix("*.").unwrap_or(host);
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_nested_and_flat_view_uris() {
        let nested = json!({"ui": {"resourceUri": "ui://sentry/chart.html"}});
        assert_eq!(tool_view_uri(Some(&nested)), Some("ui://sentry/chart.html"));
        let flat = json!({"ui/resourceUri": "ui://weather/view"});
        assert_eq!(tool_view_uri(Some(&flat)), Some("ui://weather/view"));
        assert_eq!(tool_view_uri(None), None);
    }

    #[test]
    fn rejects_views_outside_the_ui_scheme() {
        for uri in [
            "https://evil.example/app.html",
            "ui://",
            "data:text/html,<script>",
            "ui://x\nInjected",
        ] {
            let meta = json!({"ui": {"resourceUri": uri}});
            assert_eq!(tool_view_uri(Some(&meta)), None, "{uri}");
        }
        let long = format!("ui://{}", "a".repeat(600));
        assert_eq!(
            tool_view_uri(Some(&json!({"ui": {"resourceUri": long}}))),
            None
        );
    }

    #[test]
    fn app_only_tools_lack_model_visibility() {
        assert!(tool_is_app_only(Some(
            &json!({"ui": {"visibility": ["app"]}})
        )));
        assert!(!tool_is_app_only(Some(
            &json!({"ui": {"visibility": ["model", "app"]}})
        )));
        assert!(!tool_is_app_only(Some(&json!({"ui": {}}))));
        assert!(!tool_is_app_only(None));
    }

    #[test]
    fn call_details_round_trip_and_reject_bad_uris() {
        let details = json!({MCP_APP_DETAILS_KEY: {
            "server": "sentry",
            "tool": "render_sentry_chart",
            "resource_uri": "ui://sentry/chart.html",
            "structured_content": {"events": 3}
        }});
        let parsed = McpAppCallDetails::from_details(Some(&details)).unwrap();
        assert_eq!(parsed.server, "sentry");
        assert_eq!(parsed.structured_content, Some(json!({"events": 3})));

        let bad = json!({MCP_APP_DETAILS_KEY: {
            "server": "sentry", "tool": "t", "resource_uri": "https://evil.example"
        }});
        assert!(McpAppCallDetails::from_details(Some(&bad)).is_none());
    }

    #[test]
    fn view_policy_keeps_only_https_origins() {
        let meta = json!({"ui": {
            "csp": {
                "connectDomains": ["https://sentry.io", "*", "http://plain.example", "data:"],
                "resourceDomains": ["https://*.sentry-cdn.com", "https://bad host"],
            },
            "prefersBorder": true
        }});
        let policy = McpAppViewPolicy::from_meta(Some(&meta));
        assert_eq!(policy.connect_domains, vec!["https://sentry.io"]);
        assert_eq!(policy.resource_domains, vec!["https://*.sentry-cdn.com"]);
        assert!(policy.frame_domains.is_empty());
        assert_eq!(policy.prefers_border, Some(true));
        assert_eq!(
            McpAppViewPolicy::from_meta(None),
            McpAppViewPolicy::default()
        );
    }
}
