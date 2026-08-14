//! `resource_discovery` capability configuration.
//!
//! Mirrors the capability config block in the spec: an allowlist of registries
//! (the model selects a `registry_id`, never a raw URL), a trust gate, an
//! attachment type allowlist, an attachment cap, and a local-URL escape hatch
//! for tests.

use everruns_provider::url_validation::validate_safe_url;
use serde::{Deserialize, Serialize};

use crate::client::{Federation, MEDIA_TYPE_A2A_AGENT_CARD, MEDIA_TYPE_MCP_SERVER};

/// Default cap on attachments per session.
pub const DEFAULT_MAX_ATTACHMENTS: usize = 5;

/// One allowlisted registry the model may query by `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Stable id the model selects in `discover_resources`/`attach_resource`.
    pub id: String,
    /// Base URL of the registry REST API (e.g. `https://.../api/v1`).
    pub url: String,
    /// Federation mode requested on searches against this registry.
    #[serde(default)]
    pub federation: Federation,
}

/// Parsed `resource_discovery` capability config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArdConfig {
    #[serde(default)]
    pub registries: Vec<RegistryConfig>,
    /// Required attestation types (e.g. `["soc2"]`) enforced before any attach.
    #[serde(default)]
    pub require_trust: Vec<String>,
    /// Media types permitted for attachment.
    #[serde(default = "default_allow_attach_types")]
    pub allow_attach_types: Vec<String>,
    /// Maximum attachments per session.
    #[serde(default = "default_max_attachments")]
    pub max_attachments: usize,
    /// Allow attaching/ resolving loopback/private URLs. Tests/dev only.
    #[serde(default)]
    pub allow_local_urls: bool,
}

fn default_allow_attach_types() -> Vec<String> {
    vec![
        MEDIA_TYPE_MCP_SERVER.to_string(),
        MEDIA_TYPE_A2A_AGENT_CARD.to_string(),
    ]
}

fn default_max_attachments() -> usize {
    DEFAULT_MAX_ATTACHMENTS
}

impl Default for ArdConfig {
    fn default() -> Self {
        Self {
            registries: Vec::new(),
            require_trust: Vec::new(),
            allow_attach_types: default_allow_attach_types(),
            max_attachments: DEFAULT_MAX_ATTACHMENTS,
            allow_local_urls: false,
        }
    }
}

impl ArdConfig {
    /// Parse from capability config JSON. `null`/absent yields defaults.
    pub fn from_value(value: &serde_json::Value) -> Result<Self, String> {
        if value.is_null() {
            return Ok(Self::default());
        }
        serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid resource_discovery config: {e}"))
    }

    /// Validate config invariants (called from `Capability::validate_config`).
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for reg in &self.registries {
            if reg.id.trim().is_empty() {
                return Err("registry id cannot be empty".to_string());
            }
            if !seen.insert(reg.id.clone()) {
                return Err(format!("duplicate registry id '{}'", reg.id));
            }
            // Registry URLs are operator-provided, but still drive server-side
            // HTTP. Reject static SSRF targets unless explicitly enabled for
            // local/dev registries; runtime DNS pinning happens before search.
            if self.allow_local_urls {
                let parsed = url::Url::parse(&reg.url)
                    .map_err(|e| format!("registry '{}' has invalid url: {e}", reg.id))?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Err(format!("registry '{}' url must be http(s)", reg.id));
                }
            } else if let Err(e) = validate_safe_url(&reg.url) {
                return Err(format!(
                    "registry '{}' url blocked by SSRF policy: {e}",
                    reg.id
                ));
            }
        }
        if self.max_attachments == 0 {
            return Err("max_attachments must be at least 1".to_string());
        }
        for t in &self.allow_attach_types {
            if t != MEDIA_TYPE_MCP_SERVER && t != MEDIA_TYPE_A2A_AGENT_CARD {
                return Err(format!("unsupported allow_attach_type '{t}'"));
            }
        }
        Ok(())
    }

    /// Look up a configured registry by id.
    pub fn registry(&self, id: &str) -> Option<&RegistryConfig> {
        self.registries.iter().find(|r| r.id == id)
    }

    /// Whether a media type may be attached under this config.
    pub fn allows_type(&self, media_type: &str) -> bool {
        self.allow_attach_types.iter().any(|t| t == media_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = ArdConfig::default();
        assert_eq!(cfg.max_attachments, DEFAULT_MAX_ATTACHMENTS);
        assert!(!cfg.allow_local_urls);
        assert!(cfg.allows_type(MEDIA_TYPE_MCP_SERVER));
        assert!(cfg.allows_type(MEDIA_TYPE_A2A_AGENT_CARD));
    }

    #[test]
    fn parses_full_config() {
        let cfg = ArdConfig::from_value(&serde_json::json!({
            "registries": [
                {"id": "enterprise", "url": "https://registry.acme.com/api/v1", "federation": "referrals"},
                {"id": "public", "url": "https://agenticresourcediscovery.org/api/v1", "federation": "none"}
            ],
            "require_trust": ["soc2"],
            "allow_attach_types": ["application/mcp-server+json"],
            "max_attachments": 3,
            "allow_local_urls": false
        }))
        .unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.registries.len(), 2);
        assert_eq!(cfg.registry("public").unwrap().federation, Federation::None);
        assert!(cfg.allows_type(MEDIA_TYPE_MCP_SERVER));
        assert!(!cfg.allows_type(MEDIA_TYPE_A2A_AGENT_CARD));
    }

    #[test]
    fn rejects_duplicate_registry_id() {
        let cfg = ArdConfig::from_value(&serde_json::json!({
            "registries": [
                {"id": "a", "url": "https://x.com"},
                {"id": "a", "url": "https://y.com"}
            ]
        }))
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_local_registry_urls_unless_allowed() {
        let cfg = ArdConfig::from_value(&serde_json::json!({
            "registries": [{"id": "local", "url": "http://127.0.0.1:8080"}],
            "allow_local_urls": false
        }))
        .unwrap();
        assert!(cfg.validate().unwrap_err().contains("SSRF policy"));

        let cfg = ArdConfig::from_value(&serde_json::json!({
            "registries": [{"id": "local", "url": "http://127.0.0.1:8080"}],
            "allow_local_urls": true
        }))
        .unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_zero_max_attachments() {
        let cfg = ArdConfig::from_value(&serde_json::json!({"max_attachments": 0})).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_unknown_attach_type() {
        let cfg =
            ArdConfig::from_value(&serde_json::json!({"allow_attach_types": ["application/x"]}))
                .unwrap();
        assert!(cfg.validate().is_err());
    }
}
