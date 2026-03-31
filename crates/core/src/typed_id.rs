// Typed ID module - provides type-safe, prefixed identifiers
// See specs/id-schema.md for the full specification
//
// Design decisions:
// - Uses marker traits to differentiate ID types at compile time
// - Stores IDs as prefixed strings (e.g., "agent_01933b5a...")
// - Uses UUIDv7 for time-ordering benefits
// - Supports serde, sqlx, and utoipa for full integration

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::str::FromStr;
use uuid::Uuid;

/// Error type for ID parsing failures
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdParseError {
    /// ID doesn't start with the expected prefix
    InvalidPrefix { expected: &'static str, got: String },
    /// Suffix is not valid hex
    InvalidHex(String),
    /// Suffix has wrong length
    InvalidLength { expected: usize, got: usize },
}

impl std::fmt::Display for IdParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdParseError::InvalidPrefix { expected, got } => {
                write!(f, "invalid prefix: expected '{}', got '{}'", expected, got)
            }
            IdParseError::InvalidHex(s) => write!(f, "invalid hex in ID: {}", s),
            IdParseError::InvalidLength { expected, got } => {
                write!(f, "invalid length: expected {}, got {}", expected, got)
            }
        }
    }
}

impl std::error::Error for IdParseError {}

/// Marker trait for typed ID types
pub trait IdMarker: Clone + Copy + Send + Sync + 'static {
    /// The prefix for this ID type (e.g., "agt" for agents)
    const PREFIX: &'static str;
}

/// A type-safe identifier with a specific prefix
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypedId<T: IdMarker> {
    uuid: Uuid,
    _marker: PhantomData<T>,
}

impl<T: IdMarker> TypedId<T> {
    /// Create a new ID with a random UUIDv7
    pub fn new() -> Self {
        Self {
            uuid: Uuid::now_v7(),
            _marker: PhantomData,
        }
    }

    /// Create an ID from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self {
            uuid,
            _marker: PhantomData,
        }
    }

    /// Get the underlying UUID
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Get the prefix for this ID type
    pub fn prefix() -> &'static str {
        T::PREFIX
    }

    /// Parse an ID from a prefixed string
    pub fn parse(s: &str) -> Result<Self, IdParseError> {
        let expected_prefix = format!("{}_", T::PREFIX);

        if !s.starts_with(&expected_prefix) {
            let got_prefix = s.split('_').next().unwrap_or("").to_string();
            return Err(IdParseError::InvalidPrefix {
                expected: T::PREFIX,
                got: got_prefix,
            });
        }

        let suffix = &s[expected_prefix.len()..];

        if suffix.len() != 32 {
            return Err(IdParseError::InvalidLength {
                expected: 32,
                got: suffix.len(),
            });
        }

        // Validate hex characters (lowercase only)
        if !suffix
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(IdParseError::InvalidHex(suffix.to_string()));
        }

        // Parse the UUID
        let uuid =
            Uuid::parse_str(suffix).map_err(|_| IdParseError::InvalidHex(suffix.to_string()))?;

        Ok(Self {
            uuid,
            _marker: PhantomData,
        })
    }

    /// Create an ID from a well-known integer value (for seeding)
    /// Produces IDs like "agent_00000000000000000000000000000001"
    pub fn from_seed(value: u128) -> Self {
        let uuid = Uuid::from_u128(value);
        Self {
            uuid,
            _marker: PhantomData,
        }
    }
}

impl<T: IdMarker> Default for TypedId<T> {
    fn default() -> Self {
        Self::new()
    }
}

// Conversion from TypedId to Uuid
impl<T: IdMarker> From<TypedId<T>> for Uuid {
    fn from(id: TypedId<T>) -> Self {
        id.uuid
    }
}

// Conversion from Uuid to TypedId
impl<T: IdMarker> From<Uuid> for TypedId<T> {
    fn from(uuid: Uuid) -> Self {
        Self::from_uuid(uuid)
    }
}

// Allow using TypedId as a key in HashMap/HashSet that expects Uuid
impl<T: IdMarker> std::borrow::Borrow<Uuid> for TypedId<T> {
    fn borrow(&self) -> &Uuid {
        &self.uuid
    }
}

// AsRef<Uuid> for TypedId
impl<T: IdMarker> AsRef<Uuid> for TypedId<T> {
    fn as_ref(&self) -> &Uuid {
        &self.uuid
    }
}

// PartialEq<Uuid> for TypedId (allows comparing TypedId == Uuid)
impl<T: IdMarker> PartialEq<Uuid> for TypedId<T> {
    fn eq(&self, other: &Uuid) -> bool {
        self.uuid == *other
    }
}

// PartialEq<TypedId> for Uuid (allows comparing Uuid == TypedId)
impl<T: IdMarker> PartialEq<TypedId<T>> for Uuid {
    fn eq(&self, other: &TypedId<T>) -> bool {
        *self == other.uuid
    }
}

impl<T: IdMarker> fmt::Display for TypedId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", T::PREFIX, self.uuid.simple())
    }
}

impl<T: IdMarker> fmt::Debug for TypedId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}({})",
            std::any::type_name::<T>()
                .split("::")
                .last()
                .unwrap_or("Id"),
            self
        )
    }
}

impl<T: IdMarker> FromStr for TypedId<T> {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl<T: IdMarker> Serialize for TypedId<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de, T: IdMarker> Deserialize<'de> for TypedId<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

// OpenAPI schema support
#[cfg(feature = "openapi")]
impl<T: IdMarker> utoipa::ToSchema for TypedId<T> {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("TypedId")
    }
}

#[cfg(feature = "openapi")]
impl<T: IdMarker> utoipa::PartialSchema for TypedId<T> {
    #[allow(deprecated)]
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        let example_value = format!("{}_{}", T::PREFIX, "01933b5a00007000800000000000001");
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some(format!(
                "Prefixed identifier with '{}' prefix",
                T::PREFIX
            )))
            .example(Some(serde_json::json!(example_value)))
            .pattern(Some(format!("^{}_[0-9a-f]{{32}}$", T::PREFIX)))
            .into()
    }
}

// sqlx support - maps TypedId to/from UUID in database
#[cfg(feature = "sqlx")]
impl<T: IdMarker> sqlx::Type<sqlx::Postgres> for TypedId<T> {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <Uuid as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <Uuid as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

#[cfg(feature = "sqlx")]
impl<T: IdMarker> sqlx::Encode<'_, sqlx::Postgres> for TypedId<T> {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        <Uuid as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.uuid, buf)
    }
}

#[cfg(feature = "sqlx")]
impl<T: IdMarker> sqlx::Decode<'_, sqlx::Postgres> for TypedId<T> {
    fn decode(
        value: sqlx::postgres::PgValueRef<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let uuid = <Uuid as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(Self::from_uuid(uuid))
    }
}

// ============================================================================
// Marker types for each entity
// ============================================================================

/// Marker for Organization IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OrgIdMarker;
impl IdMarker for OrgIdMarker {
    const PREFIX: &'static str = "org";
}

/// Marker for Agent IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AgentIdMarker;
impl IdMarker for AgentIdMarker {
    const PREFIX: &'static str = "agent";
}

/// Marker for Harness IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HarnessIdMarker;
impl IdMarker for HarnessIdMarker {
    const PREFIX: &'static str = "harness";
}

/// Marker for agent identity IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AgentIdentityIdMarker;
impl IdMarker for AgentIdentityIdMarker {
    const PREFIX: &'static str = "identity";
}

/// Marker for Session IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionIdMarker;
impl IdMarker for SessionIdMarker {
    const PREFIX: &'static str = "session";
}

/// Marker for Message IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageIdMarker;
impl IdMarker for MessageIdMarker {
    const PREFIX: &'static str = "message";
}

/// Marker for Event IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventIdMarker;
impl IdMarker for EventIdMarker {
    const PREFIX: &'static str = "event";
}

/// Marker for LLM Provider IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProviderIdMarker;
impl IdMarker for ProviderIdMarker {
    const PREFIX: &'static str = "provider";
}

/// Marker for LLM Model IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelIdMarker;
impl IdMarker for ModelIdMarker {
    const PREFIX: &'static str = "model";
}

/// Marker for Image IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageIdMarker;
impl IdMarker for ImageIdMarker {
    const PREFIX: &'static str = "img";
}

/// Marker for MCP Server IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct McpServerIdMarker;
impl IdMarker for McpServerIdMarker {
    const PREFIX: &'static str = "mcp";
}

/// Marker for Skill IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SkillIdMarker;
impl IdMarker for SkillIdMarker {
    const PREFIX: &'static str = "skill";
}

/// Marker for Turn IDs (used in workflow execution)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TurnIdMarker;
impl IdMarker for TurnIdMarker {
    const PREFIX: &'static str = "turn";
}

/// Marker for Execution IDs (workflow execution instance)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecIdMarker;
impl IdMarker for ExecIdMarker {
    const PREFIX: &'static str = "exec";
}

/// Marker for Session Schedule IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScheduleIdMarker;
impl IdMarker for ScheduleIdMarker {
    const PREFIX: &'static str = "sched";
}

/// Marker for leased resource IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LeasedResourceIdMarker;
impl IdMarker for LeasedResourceIdMarker {
    const PREFIX: &'static str = "resource";
}

/// Marker for App IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AppIdMarker;
impl IdMarker for AppIdMarker {
    const PREFIX: &'static str = "app";
}

/// Marker for App Channel IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AppChannelIdMarker;
impl IdMarker for AppChannelIdMarker {
    const PREFIX: &'static str = "appchan";
}

/// Marker for Notification IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationIdMarker;
impl IdMarker for NotificationIdMarker {
    const PREFIX: &'static str = "notification";
}

/// Marker for Memory Store IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryStoreIdMarker;
impl IdMarker for MemoryStoreIdMarker {
    const PREFIX: &'static str = "mst";
}

/// Marker for Memory IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryIdMarker;
impl IdMarker for MemoryIdMarker {
    const PREFIX: &'static str = "mem";
}

/// Marker for Eval IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EvalIdMarker;
impl IdMarker for EvalIdMarker {
    const PREFIX: &'static str = "eval";
}

/// Marker for Eval Case IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EvalCaseIdMarker;
impl IdMarker for EvalCaseIdMarker {
    const PREFIX: &'static str = "evalcase";
}

/// Marker for Eval Run IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EvalRunIdMarker;
impl IdMarker for EvalRunIdMarker {
    const PREFIX: &'static str = "evalrun";
}

/// Marker for Eval Case Result IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EvalResultIdMarker;
impl IdMarker for EvalResultIdMarker {
    const PREFIX: &'static str = "evalresult";
}

/// Marker for Budget IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BudgetIdMarker;
impl IdMarker for BudgetIdMarker {
    const PREFIX: &'static str = "bdgt";
}

/// Marker for Budget Ledger Entry IDs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LedgerEntryIdMarker;
impl IdMarker for LedgerEntryIdMarker {
    const PREFIX: &'static str = "ledger";
}

// ============================================================================
// Type aliases for convenience
// ============================================================================

/// Organization ID
pub type OrgId = TypedId<OrgIdMarker>;
/// Agent ID
pub type AgentId = TypedId<AgentIdMarker>;
/// Harness ID
pub type HarnessId = TypedId<HarnessIdMarker>;
/// Agent identity ID
pub type AgentIdentityId = TypedId<AgentIdentityIdMarker>;
/// Session ID
pub type SessionId = TypedId<SessionIdMarker>;
/// Message ID
pub type MessageId = TypedId<MessageIdMarker>;
/// Event ID
pub type EventId = TypedId<EventIdMarker>;
/// LLM Provider ID
pub type ProviderId = TypedId<ProviderIdMarker>;
/// LLM Model ID
pub type ModelId = TypedId<ModelIdMarker>;
/// Image ID
pub type ImageId = TypedId<ImageIdMarker>;
/// MCP Server ID
pub type McpServerId = TypedId<McpServerIdMarker>;
/// Skill ID
pub type SkillId = TypedId<SkillIdMarker>;
/// Turn ID
pub type TurnId = TypedId<TurnIdMarker>;
/// Execution ID
pub type ExecId = TypedId<ExecIdMarker>;
/// Session Schedule ID
pub type ScheduleId = TypedId<ScheduleIdMarker>;
/// Leased resource ID
pub type LeasedResourceId = TypedId<LeasedResourceIdMarker>;
/// App ID
pub type AppId = TypedId<AppIdMarker>;
/// App Channel ID
pub type AppChannelId = TypedId<AppChannelIdMarker>;
/// Notification ID
pub type NotificationId = TypedId<NotificationIdMarker>;
pub type MemoryStoreId = TypedId<MemoryStoreIdMarker>;
pub type MemoryId = TypedId<MemoryIdMarker>;
/// Eval ID
pub type EvalId = TypedId<EvalIdMarker>;
/// Eval Case ID
pub type EvalCaseId = TypedId<EvalCaseIdMarker>;
/// Eval Run ID
pub type EvalRunId = TypedId<EvalRunIdMarker>;
/// Eval Case Result ID
pub type EvalResultId = TypedId<EvalResultIdMarker>;
/// Budget ID
pub type BudgetId = TypedId<BudgetIdMarker>;
/// Budget Ledger Entry ID
pub type LedgerEntryId = TypedId<LedgerEntryIdMarker>;

// ============================================================================
// Well-known IDs (for seeding and defaults)
// ============================================================================

/// Default organization ID
pub const DEFAULT_ORG_ID: OrgId = TypedId {
    uuid: Uuid::from_u128(1),
    _marker: PhantomData,
};

/// Well-known provider IDs
pub mod well_known {
    use super::*;

    /// OpenAI provider ID
    pub const OPENAI_PROVIDER_ID: ProviderId = TypedId {
        uuid: Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000001),
        _marker: PhantomData,
    };

    /// Anthropic provider ID
    pub const ANTHROPIC_PROVIDER_ID: ProviderId = TypedId {
        uuid: Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000002),
        _marker: PhantomData,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_agent_id() {
        let id = AgentId::new();
        let s = id.to_string();
        assert!(s.starts_with("agent_"));
        assert_eq!(s.len(), 38); // "agent_" + 32 hex chars
    }

    #[test]
    fn test_parse_agent_id() {
        let id = AgentId::new();
        let s = id.to_string();
        let parsed = AgentId::parse(&s).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_parse_invalid_prefix() {
        let result = AgentId::parse("session_01933b5a00007000800000000000001");
        assert!(matches!(result, Err(IdParseError::InvalidPrefix { .. })));
    }

    #[test]
    fn test_parse_invalid_length() {
        let result = AgentId::parse("agent_123");
        assert!(matches!(result, Err(IdParseError::InvalidLength { .. })));
    }

    #[test]
    fn test_parse_invalid_hex() {
        let result = AgentId::parse("agent_GHIJKLMNOPQRSTUVWXYZ123456789012");
        assert!(matches!(result, Err(IdParseError::InvalidHex(_))));
    }

    #[test]
    fn test_from_seed() {
        let id = AgentId::from_seed(1);
        assert_eq!(id.to_string(), "agent_00000000000000000000000000000001");
    }

    #[test]
    fn test_serde_roundtrip() {
        let id = AgentId::new();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_default_org_id() {
        assert_eq!(
            DEFAULT_ORG_ID.to_string(),
            "org_00000000000000000000000000000001"
        );
    }

    #[test]
    fn test_well_known_provider_ids() {
        assert_eq!(
            well_known::OPENAI_PROVIDER_ID.to_string(),
            "provider_01933b5a000070008000000000000001"
        );
        assert_eq!(
            well_known::ANTHROPIC_PROVIDER_ID.to_string(),
            "provider_01933b5a000070008000000000000002"
        );
    }

    #[test]
    fn test_from_uuid() {
        let uuid = Uuid::now_v7();
        let id = AgentId::from_uuid(uuid);
        assert_eq!(id.uuid(), uuid);
    }

    #[test]
    fn test_hash() {
        use std::collections::HashSet;
        let id1 = AgentId::new();
        let id2 = AgentId::new();
        let mut set = HashSet::new();
        set.insert(id1);
        set.insert(id2);
        assert_eq!(set.len(), 2);
        set.insert(id1);
        assert_eq!(set.len(), 2); // No duplicate
    }
}
