//! Per-server MCP policies: which protocol era the client speaks, and which
//! elicitation modes it declares.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================================================
// MCP protocol versions and per-server adoption policy
// ============================================================================
//
// Everruns' MCP *client* speaks three protocol eras. They differ in how the
// connection is established and what metadata travels with each request:
//
// - `2025-03-26` / `2025-06-18`: *stateful*. The client must run the
//   `initialize` handshake, may receive an `Mcp-Session-Id` it has to echo on
//   every subsequent request, and sends `notifications/initialized`.
// - `2026-07-28`: *stateless*. No handshake and no session id; protocol version
//   + client info ride in `_meta` on every request, and routable headers
//   (`MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`) let edge infrastructure
//   route without parsing the body.
//
// Eras are named by their version date, not by a moving label like "stable" or
// "rc" — `2026-07-28` shipped as a final spec on 2026-07-28, and the previous
// naming outlived its meaning within one release.
//
// See knowledge/integrations/mcp-servers.md (Multi-era protocol support) and the negotiation
// engine in `everruns-mcp` (`protocol.rs`).

/// MCP `2025-03-26` (stateful handshake). Oldest era the client speaks.
pub const MCP_PROTOCOL_VERSION_2025_03: &str = "2025-03-26";
/// MCP `2025-06-18` (stateful handshake).
pub const MCP_PROTOCOL_VERSION_2025_06: &str = "2025-06-18";
/// MCP `2026-07-28` (stateless). Current era.
pub const MCP_PROTOCOL_VERSION_2026_07: &str = "2026-07-28";

/// Per-server policy for which MCP protocol era the client uses.
///
/// `Auto` (the default) probes the server and adapts — it tries the stateless
/// `2026-07-28` path first and transparently falls back to the stateful
/// handshake when a server demands it, so a single configuration speaks to
/// every era without operator action. The pinned variants skip negotiation when
/// an operator knows a server's era (or to work around a server that
/// mis-signals it).
///
/// Wire values are the version dates. The pre-release names (`legacy`,
/// `stable`, `rc`) stay accepted as deserialization aliases so stored config
/// keeps loading, but they are no longer emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "auto"))]
#[serde(rename_all = "snake_case")]
pub enum McpProtocolMode {
    /// Probe once, detect the server's era, adapt, and cache the verdict.
    #[default]
    Auto,
    /// Pin to `2025-03-26` stateful behavior (handshake + session id).
    #[serde(rename = "2025-03-26", alias = "legacy")]
    V2025March,
    /// Pin to `2025-06-18` stateful behavior (handshake + session id).
    #[serde(rename = "2025-06-18", alias = "stable")]
    V2025June,
    /// Pin to `2026-07-28` stateless behavior (`_meta` per request, routable
    /// headers, no handshake).
    #[serde(rename = "2026-07-28", alias = "rc")]
    V2026July,
}

impl McpProtocolMode {
    /// Whether this is the default `Auto` policy. Used to keep the field out of
    /// serialized config when it carries no information.
    pub fn is_auto(&self) -> bool {
        matches!(self, McpProtocolMode::Auto)
    }

    /// The protocol version string a *pinned* mode advertises. `Auto` returns
    /// `None` because its version is decided by negotiation at runtime.
    pub fn pinned_version(&self) -> Option<&'static str> {
        match self {
            McpProtocolMode::Auto => None,
            McpProtocolMode::V2025March => Some(MCP_PROTOCOL_VERSION_2025_03),
            McpProtocolMode::V2025June => Some(MCP_PROTOCOL_VERSION_2025_06),
            McpProtocolMode::V2026July => Some(MCP_PROTOCOL_VERSION_2026_07),
        }
    }

    /// Whether a pinned mode requires the stateful `initialize` handshake.
    /// `Auto` returns `None` (decided by negotiation).
    pub fn pinned_stateful(&self) -> Option<bool> {
        match self {
            McpProtocolMode::Auto => None,
            McpProtocolMode::V2025March | McpProtocolMode::V2025June => Some(true),
            McpProtocolMode::V2026July => Some(false),
        }
    }
}

impl std::fmt::Display for McpProtocolMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpProtocolMode::Auto => write!(f, "auto"),
            McpProtocolMode::V2025March => write!(f, "{MCP_PROTOCOL_VERSION_2025_03}"),
            McpProtocolMode::V2025June => write!(f, "{MCP_PROTOCOL_VERSION_2025_06}"),
            McpProtocolMode::V2026July => write!(f, "{MCP_PROTOCOL_VERSION_2026_07}"),
        }
    }
}

impl From<&str> for McpProtocolMode {
    /// Parses the canonical version-date values and the pre-release aliases
    /// (`legacy`/`stable`/`rc`) that stored config and older workers still send.
    /// Anything unrecognized falls back to `Auto`, which negotiates anyway.
    fn from(s: &str) -> Self {
        match s {
            MCP_PROTOCOL_VERSION_2025_03 | "legacy" => McpProtocolMode::V2025March,
            MCP_PROTOCOL_VERSION_2025_06 | "stable" => McpProtocolMode::V2025June,
            MCP_PROTOCOL_VERSION_2026_07 | "rc" => McpProtocolMode::V2026July,
            _ => McpProtocolMode::Auto,
        }
    }
}

/// Which MCP elicitation modes the client declares to one server.
///
/// An operator decision on the server record, never a per-call negotiation and
/// never something the model can widen. Under MRTR a server must not ask for an
/// input type the client did not declare, so this is what stops a configured
/// server from putting questions in front of a person.
///
/// THREAT[TM-TOOL-045]: form mode is opt-in per server. The default keeps the
/// behaviour every existing deployment already had, and it is omitted from
/// serialized config so stored records stay byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "url"))]
#[serde(rename_all = "snake_case")]
pub enum McpElicitationPolicy {
    /// Declare URL mode only: the server may ask a person to open a link.
    #[default]
    Url,
    /// Also declare form mode: the server may ask a person structured
    /// questions, answered through the session's `ask_user` surface.
    UrlAndForm,
    /// Declare no elicitation at all.
    None,
}

impl McpElicitationPolicy {
    /// Whether this is the default policy. Keeps the field out of serialized
    /// config when it carries no information.
    pub fn is_default(&self) -> bool {
        matches!(self, McpElicitationPolicy::Url)
    }

    /// Whether URL mode may be declared to this server.
    pub fn allows_url(&self) -> bool {
        matches!(
            self,
            McpElicitationPolicy::Url | McpElicitationPolicy::UrlAndForm
        )
    }

    /// Whether form mode may be declared to this server.
    pub fn allows_form(&self) -> bool {
        matches!(self, McpElicitationPolicy::UrlAndForm)
    }
}

impl std::fmt::Display for McpElicitationPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpElicitationPolicy::Url => write!(f, "url"),
            McpElicitationPolicy::UrlAndForm => write!(f, "url_and_form"),
            McpElicitationPolicy::None => write!(f, "none"),
        }
    }
}

impl From<&str> for McpElicitationPolicy {
    /// Anything unrecognized falls back to the default, which is exactly what
    /// every server had before the policy existed.
    fn from(s: &str) -> Self {
        match s {
            "url_and_form" => McpElicitationPolicy::UrlAndForm,
            "none" => McpElicitationPolicy::None,
            _ => McpElicitationPolicy::Url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::ScopedMcpServer;
    use serde_json::json;

    #[test]
    fn elicitation_policy_defaults_to_url_and_stays_out_of_serialized_config() {
        let scoped: ScopedMcpServer =
            serde_json::from_value(json!({"type":"http","url":"https://example.com/mcp"})).unwrap();
        assert_eq!(scoped.elicitation_policy, McpElicitationPolicy::Url);
        let wire = serde_json::to_value(&scoped).unwrap();
        assert!(
            wire.get("elicitation_policy").is_none(),
            "default policy must keep stored config byte-identical: {wire}"
        );

        for (value, policy) in [
            ("url_and_form", McpElicitationPolicy::UrlAndForm),
            ("none", McpElicitationPolicy::None),
        ] {
            let scoped: ScopedMcpServer = serde_json::from_value(json!({
                "type":"http","url":"https://example.com/mcp","elicitation_policy":value
            }))
            .unwrap();
            assert_eq!(scoped.elicitation_policy, policy);
            assert_eq!(
                serde_json::to_value(&scoped).unwrap()["elicitation_policy"],
                json!(value)
            );
            assert_eq!(McpElicitationPolicy::from(value), policy);
        }

        assert!(McpElicitationPolicy::Url.allows_url());
        assert!(!McpElicitationPolicy::Url.allows_form());
        assert!(McpElicitationPolicy::UrlAndForm.allows_form());
        assert!(!McpElicitationPolicy::None.allows_url());
        assert!(!McpElicitationPolicy::None.allows_form());
        // An unknown wire value from an older or newer peer degrades to the
        // default rather than widening what a server may ask.
        assert_eq!(
            McpElicitationPolicy::from("everything"),
            McpElicitationPolicy::Url
        );
    }
}
