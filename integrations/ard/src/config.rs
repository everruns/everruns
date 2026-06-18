//! Configuration, ARD envelope types, URN parsing, and trust verification.
//!
//! All types here treat registry-returned data as **untrusted** external input.
//! The model never supplies a raw URL — it picks a configured `registry_id`, and
//! resolved URLs are SSRF-validated before any outbound use (see `tools.rs`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Media types -> attachment kinds
// ============================================================================

/// ARD media type for an MCP server descriptor.
pub const MEDIA_TYPE_MCP_SERVER: &str = "application/mcp-server+json";
/// ARD media type for an A2A agent card.
pub const MEDIA_TYPE_A2A_AGENT_CARD: &str = "application/a2a-agent-card+json";

/// What an attached resource becomes inside the session config layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    /// Session-scoped MCP server (`mcpServers`).
    McpServer,
    /// Session-scoped external A2A agent.
    A2aAgent,
}

impl AttachmentKind {
    /// The session-resource registry `kind` string used to record the attachment.
    pub fn resource_kind(self) -> &'static str {
        match self {
            Self::McpServer => "mcp_server",
            Self::A2aAgent => "external_a2a_agent",
        }
    }

    /// The configured `allow_attach_types` token a config must list to permit this.
    pub fn config_token(self) -> &'static str {
        match self {
            Self::McpServer => "mcp_server",
            Self::A2aAgent => "a2a_agent",
        }
    }

    /// Reverse of [`Self::resource_kind`]: map a recorded session-resource
    /// `kind` string back to the attachment kind. Lets repeat
    /// (`already_attached`) responses report the same `config_token` shape as a
    /// first attach.
    pub fn from_resource_kind(kind: &str) -> Option<Self> {
        match kind {
            "mcp_server" => Some(Self::McpServer),
            "external_a2a_agent" => Some(Self::A2aAgent),
            _ => None,
        }
    }
}

/// Map an ARD entry media type to the attachment kind it materializes into.
///
/// Returns `None` for any media type this increment does not support (e.g.
/// skill-type entries), which the caller must reject.
pub fn attachment_kind_for_media_type(media_type: &str) -> Option<AttachmentKind> {
    match media_type {
        MEDIA_TYPE_MCP_SERVER => Some(AttachmentKind::McpServer),
        MEDIA_TYPE_A2A_AGENT_CARD => Some(AttachmentKind::A2aAgent),
        _ => None,
    }
}

// ============================================================================
// Capability config
// ============================================================================

/// A single operator-configured ARD registry.
///
/// The model references registries by `id` only — it can NEVER supply a raw
/// registry URL. This is the primary SSRF/exfiltration control.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryConfig {
    /// Stable identifier the model uses in `discover_resources`/`attach_resource`.
    pub id: String,
    /// Operator-provided base URL of the ARD registry (e.g. `https://ard.example.com`).
    pub url: String,
    /// RESERVED — NOT YET ENFORCED. Intended as a federation allowlist of
    /// registry hostnames this registry may delegate resolution to. The client
    /// does not follow delegated/federated registry links today (it only talks
    /// to the configured `url`), so this field has no security effect yet; it
    /// is parsed and preserved for forward compatibility. Wiring it up is a
    /// documented follow-up (see SPEC.md).
    #[serde(default)]
    pub federation: Vec<String>,
}

/// Per-capability config for `resource_discovery`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDiscoveryConfig {
    /// Operator-configured registries. The model selects one by `id`.
    #[serde(default)]
    pub registries: Vec<RegistryConfig>,
    /// Require a verifiable trust manifest before any attach. Defaults true.
    #[serde(default = "default_true")]
    pub require_trust: bool,
    /// Attachment kinds permitted. Tokens: "mcp_server", "a2a_agent".
    /// Empty means "all supported kinds".
    #[serde(default)]
    pub allow_attach_types: Vec<String>,
    /// Maximum number of resources that may be attached in a session.
    #[serde(default = "default_max_attachments")]
    pub max_attachments: usize,
    /// Allow resolved URLs that point at local/private addresses. Defaults false.
    /// This is a test/dev escape hatch and bypasses ONLY local-URL blocking.
    #[serde(default)]
    pub allow_local_urls: bool,
}

fn default_true() -> bool {
    true
}

fn default_max_attachments() -> usize {
    8
}

impl Default for ResourceDiscoveryConfig {
    fn default() -> Self {
        Self {
            registries: Vec::new(),
            require_trust: true,
            allow_attach_types: Vec::new(),
            max_attachments: default_max_attachments(),
            allow_local_urls: false,
        }
    }
}

impl ResourceDiscoveryConfig {
    /// Parse config from the per-capability JSON value, defaulting on error.
    pub fn from_value(value: &Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    /// Look up a configured registry by id.
    pub fn registry(&self, id: &str) -> Option<&RegistryConfig> {
        self.registries.iter().find(|r| r.id == id)
    }

    /// Returns whether the given attachment kind is permitted by config.
    pub fn allows_kind(&self, kind: AttachmentKind) -> bool {
        self.allow_attach_types.is_empty()
            || self
                .allow_attach_types
                .iter()
                .any(|t| t == kind.config_token())
    }
}

// ============================================================================
// ARD envelope (untrusted registry response shapes)
// ============================================================================

/// A ranked search hit from `POST /search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub urn: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// ARD media type (e.g. `application/mcp-server+json`).
    #[serde(rename = "type", default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Response body of `POST /search`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResponse {
    #[serde(default)]
    pub results: Vec<SearchHit>,
}

/// A trust manifest binding an entry to a verifiable identity.
///
/// Verification in this increment is structural: the `identity` domain must
/// match the entry URN's authority domain (URN domain <-> identity), and an
/// attestation must be present. Cryptographic signature verification is a
/// documented follow-up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustManifest {
    /// Verified identity, e.g. a domain like `example.com`.
    pub identity: String,
    /// Attestation blob (signature/JWT/etc.). Presence is required; full
    /// cryptographic verification is a follow-up.
    #[serde(default)]
    pub attestation: Option<String>,
}

/// A resolved ARD entry envelope from `GET`/`POST` resolve on a registry.
///
/// The envelope is **value-or-reference**: it carries either an inline `data`
/// payload OR a `url` reference, never both. Entries with both are rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedEntry {
    pub urn: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// ARD media type.
    #[serde(rename = "type")]
    pub media_type: String,
    /// Reference URL to the resource descriptor (value-or-reference).
    #[serde(default)]
    pub url: Option<String>,
    /// Inline resource descriptor (value-or-reference).
    #[serde(default)]
    pub data: Option<Value>,
    /// Trust manifest, if the registry provided one.
    #[serde(default, rename = "trustManifest")]
    pub trust_manifest: Option<TrustManifest>,
}

/// Errors arising from envelope/trust validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// The envelope carried both `url` and `data`.
    ValueAndReference,
    /// The envelope carried neither `url` nor `data`.
    NeitherValueNorReference,
    /// URN could not be parsed.
    InvalidUrn(String),
    /// Unsupported media type for this increment.
    UnsupportedMediaType(String),
    /// Trust required but the manifest was missing.
    MissingTrustManifest,
    /// Trust required but no attestation was present.
    MissingAttestation,
    /// The trust identity does not match the URN domain.
    IdentityMismatch {
        urn_domain: String,
        identity: String,
    },
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ValueAndReference => {
                write!(
                    f,
                    "ARD entry carries both `url` and `data` (must be value-or-reference)"
                )
            }
            Self::NeitherValueNorReference => {
                write!(f, "ARD entry carries neither `url` nor `data`")
            }
            Self::InvalidUrn(u) => write!(f, "Invalid URN: {u}"),
            Self::UnsupportedMediaType(m) => write!(f, "Unsupported resource type: {m}"),
            Self::MissingTrustManifest => {
                write!(f, "Trust required but the resource has no trust manifest")
            }
            Self::MissingAttestation => {
                write!(
                    f,
                    "Trust required but the trust manifest has no attestation"
                )
            }
            Self::IdentityMismatch {
                urn_domain,
                identity,
            } => write!(
                f,
                "Trust identity `{identity}` does not match URN domain `{urn_domain}`"
            ),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl ResolvedEntry {
    /// Enforce the value-or-reference invariant.
    pub fn validate_value_or_reference(&self) -> Result<(), EnvelopeError> {
        match (self.url.is_some(), self.data.is_some()) {
            (true, true) => Err(EnvelopeError::ValueAndReference),
            (false, false) => Err(EnvelopeError::NeitherValueNorReference),
            _ => Ok(()),
        }
    }

    /// Resolve the attachment kind from the media type, rejecting unsupported types.
    pub fn attachment_kind(&self) -> Result<AttachmentKind, EnvelopeError> {
        attachment_kind_for_media_type(&self.media_type)
            .ok_or_else(|| EnvelopeError::UnsupportedMediaType(self.media_type.clone()))
    }

    /// Verify the trust manifest against `require_trust`.
    ///
    /// When `require_trust` is true: a manifest with an attestation must be
    /// present and its identity domain must match the URN authority domain.
    /// When false: a present manifest is still checked for identity match, but
    /// absence is tolerated.
    pub fn verify_trust(&self, require_trust: bool) -> Result<(), EnvelopeError> {
        let urn = Urn::parse(&self.urn)?;
        match &self.trust_manifest {
            None => {
                if require_trust {
                    Err(EnvelopeError::MissingTrustManifest)
                } else {
                    Ok(())
                }
            }
            Some(manifest) => {
                if require_trust && manifest.attestation.as_deref().unwrap_or("").is_empty() {
                    return Err(EnvelopeError::MissingAttestation);
                }
                if !urn.domain_matches(&manifest.identity) {
                    return Err(EnvelopeError::IdentityMismatch {
                        urn_domain: urn.domain().to_string(),
                        identity: manifest.identity.clone(),
                    });
                }
                Ok(())
            }
        }
    }
}

// ============================================================================
// URN parsing + domain extraction
// ============================================================================

/// A parsed ARD URN of the form `urn:ard:<domain>:<name>[:...]`.
///
/// The domain (the authority namespace) is the trust anchor: a trust manifest's
/// `identity` must correspond to this domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Urn {
    domain: String,
    name: String,
}

impl Urn {
    /// Parse a URN, extracting its domain (namespace) and name.
    ///
    /// Accepts `urn:ard:<domain>:<name>[:more]`. The domain is the third
    /// colon-delimited segment.
    pub fn parse(raw: &str) -> Result<Self, EnvelopeError> {
        let parts: Vec<&str> = raw.split(':').collect();
        // urn : ard : domain : name ...
        if parts.len() < 4
            || !parts[0].eq_ignore_ascii_case("urn")
            || !parts[1].eq_ignore_ascii_case("ard")
        {
            return Err(EnvelopeError::InvalidUrn(raw.to_string()));
        }
        let domain = parts[2].trim();
        let name = parts[3].trim();
        if domain.is_empty() || name.is_empty() {
            return Err(EnvelopeError::InvalidUrn(raw.to_string()));
        }
        Ok(Self {
            domain: domain.to_ascii_lowercase(),
            name: name.to_string(),
        })
    }

    /// The authority domain (lowercased).
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The entry name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the given identity matches this URN's domain.
    ///
    /// Match is exact (case-insensitive) or a subdomain of the identity, so an
    /// identity `example.com` covers `mcp.example.com` but not `evil.com`.
    pub fn domain_matches(&self, identity: &str) -> bool {
        let id = identity.trim().to_ascii_lowercase();
        if id.is_empty() {
            return false;
        }
        self.domain == id || self.domain.ends_with(&format!(".{id}"))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Config ---

    #[test]
    fn config_defaults() {
        let c = ResourceDiscoveryConfig::default();
        assert!(c.require_trust);
        assert!(!c.allow_local_urls);
        assert_eq!(c.max_attachments, 8);
        assert!(c.registries.is_empty());
    }

    #[test]
    fn config_from_value_roundtrip() {
        let c = ResourceDiscoveryConfig::from_value(&json!({
            "registries": [{ "id": "r1", "url": "https://r.example", "federation": ["a.example"] }],
            "require_trust": false,
            "max_attachments": 2,
            "allow_local_urls": true,
            "allow_attach_types": ["mcp_server"]
        }));
        assert_eq!(c.registries.len(), 1);
        assert_eq!(c.registry("r1").unwrap().federation, vec!["a.example"]);
        assert!(!c.require_trust);
        assert!(c.allow_local_urls);
        assert_eq!(c.max_attachments, 2);
    }

    #[test]
    fn config_allows_kind() {
        let mut c = ResourceDiscoveryConfig::default();
        // Empty => all permitted.
        assert!(c.allows_kind(AttachmentKind::McpServer));
        assert!(c.allows_kind(AttachmentKind::A2aAgent));
        c.allow_attach_types = vec!["mcp_server".into()];
        assert!(c.allows_kind(AttachmentKind::McpServer));
        assert!(!c.allows_kind(AttachmentKind::A2aAgent));
    }

    // --- Media type mapping ---

    #[test]
    fn media_type_mapping() {
        assert_eq!(
            attachment_kind_for_media_type(MEDIA_TYPE_MCP_SERVER),
            Some(AttachmentKind::McpServer)
        );
        assert_eq!(
            attachment_kind_for_media_type(MEDIA_TYPE_A2A_AGENT_CARD),
            Some(AttachmentKind::A2aAgent)
        );
        assert_eq!(
            attachment_kind_for_media_type("application/skill+json"),
            None
        );
    }

    // --- URN parsing + domain extraction ---

    #[test]
    fn urn_parses_domain_and_name() {
        let urn = Urn::parse("urn:ard:Example.com:my-server").unwrap();
        assert_eq!(urn.domain(), "example.com");
        assert_eq!(urn.name(), "my-server");
    }

    #[test]
    fn urn_parses_extra_segments() {
        let urn = Urn::parse("urn:ard:example.com:agent:v1").unwrap();
        assert_eq!(urn.domain(), "example.com");
        assert_eq!(urn.name(), "agent");
    }

    #[test]
    fn urn_rejects_malformed() {
        assert!(Urn::parse("not-a-urn").is_err());
        assert!(Urn::parse("urn:ard:example.com").is_err());
        assert!(Urn::parse("urn:other:example.com:x").is_err());
        assert!(Urn::parse("urn:ard::name").is_err());
    }

    #[test]
    fn urn_domain_match_covers_subdomains_only() {
        let urn = Urn::parse("urn:ard:mcp.example.com:s").unwrap();
        assert!(urn.domain_matches("example.com"));
        assert!(urn.domain_matches("mcp.example.com"));
        assert!(!urn.domain_matches("evil.com"));
        assert!(!urn.domain_matches("ample.com"));
        assert!(!urn.domain_matches(""));
    }

    // --- Envelope value-or-reference ---

    fn entry(url: Option<&str>, data: Option<Value>) -> ResolvedEntry {
        ResolvedEntry {
            urn: "urn:ard:example.com:foo".into(),
            display_name: None,
            media_type: MEDIA_TYPE_MCP_SERVER.into(),
            url: url.map(|s| s.to_string()),
            data,
            trust_manifest: None,
        }
    }

    #[test]
    fn envelope_rejects_both_value_and_reference() {
        let e = entry(Some("https://x.example"), Some(json!({ "a": 1 })));
        assert_eq!(
            e.validate_value_or_reference(),
            Err(EnvelopeError::ValueAndReference)
        );
    }

    #[test]
    fn envelope_rejects_neither() {
        let e = entry(None, None);
        assert_eq!(
            e.validate_value_or_reference(),
            Err(EnvelopeError::NeitherValueNorReference)
        );
    }

    #[test]
    fn envelope_accepts_reference_only() {
        assert!(
            entry(Some("https://x.example"), None)
                .validate_value_or_reference()
                .is_ok()
        );
    }

    #[test]
    fn envelope_accepts_value_only() {
        assert!(
            entry(None, Some(json!({ "a": 1 })))
                .validate_value_or_reference()
                .is_ok()
        );
    }

    #[test]
    fn unsupported_media_type_rejected() {
        let mut e = entry(Some("https://x.example"), None);
        e.media_type = "application/skill+json".into();
        assert!(matches!(
            e.attachment_kind(),
            Err(EnvelopeError::UnsupportedMediaType(_))
        ));
    }

    // --- Trust verification ---

    fn trusted_entry(identity: &str, attestation: Option<&str>) -> ResolvedEntry {
        ResolvedEntry {
            urn: "urn:ard:mcp.example.com:server".into(),
            display_name: None,
            media_type: MEDIA_TYPE_MCP_SERVER.into(),
            url: Some("https://mcp.example.com".into()),
            data: None,
            trust_manifest: Some(TrustManifest {
                identity: identity.to_string(),
                attestation: attestation.map(|s| s.to_string()),
            }),
        }
    }

    #[test]
    fn trust_passes_with_matching_identity_and_attestation() {
        let e = trusted_entry("example.com", Some("sig"));
        assert!(e.verify_trust(true).is_ok());
    }

    #[test]
    fn trust_fails_missing_manifest_when_required() {
        let e = entry(Some("https://mcp.example.com"), None);
        assert_eq!(
            e.verify_trust(true),
            Err(EnvelopeError::MissingTrustManifest)
        );
    }

    #[test]
    fn trust_tolerates_missing_manifest_when_not_required() {
        let e = entry(Some("https://mcp.example.com"), None);
        assert!(e.verify_trust(false).is_ok());
    }

    #[test]
    fn trust_fails_missing_attestation_when_required() {
        let e = trusted_entry("example.com", None);
        assert_eq!(e.verify_trust(true), Err(EnvelopeError::MissingAttestation));
    }

    #[test]
    fn trust_fails_identity_mismatch() {
        let e = trusted_entry("evil.com", Some("sig"));
        assert!(matches!(
            e.verify_trust(true),
            Err(EnvelopeError::IdentityMismatch { .. })
        ));
    }

    #[test]
    fn trust_checks_identity_even_when_not_required() {
        // A present-but-mismatched manifest is always rejected.
        let e = trusted_entry("evil.com", Some("sig"));
        assert!(e.verify_trust(false).is_err());
    }
}
