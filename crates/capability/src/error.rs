//! Structured validation and registry errors for the capability contract.

use std::fmt;

/// A structured capability contract violation.
///
/// Produced by identity/configuration validation ([`crate::validate_capability_id`],
/// [`crate::validate_capability_config`], [`crate::CapabilityRef::validate`],
/// [`crate::Definition::validate`](crate::definition::Definition::validate))
/// and by duplicate/collision rejection ([`crate::CapabilityIdIndex`],
/// [`crate::ActivationSet`]). Hosts map these onto their own error surfaces
/// (e.g. the Framework's `BuildError`, the server's HTTP 400s) without
/// re-implementing the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilityError {
    /// A capability identifier violates the open-ID grammar or reserved
    /// namespace rules.
    InvalidId {
        /// The rejected capability id.
        id: String,
        /// Why the identifier was rejected.
        reason: String,
    },
    /// A capability configuration violates the JSON object boundary.
    InvalidConfig {
        /// The capability id the configuration belongs to.
        id: String,
        /// Why the configuration was rejected.
        reason: String,
    },
    /// A code-defined capability definition is structurally invalid.
    InvalidDefinition {
        /// The rejected capability id.
        id: String,
        /// Why the definition was rejected.
        reason: String,
    },
    /// Two capability inputs resolve to the same stable identity.
    Duplicate {
        /// The colliding capability id.
        id: String,
    },
}

impl CapabilityError {
    /// The capability id the error refers to.
    pub fn id(&self) -> &str {
        match self {
            Self::InvalidId { id, .. }
            | Self::InvalidConfig { id, .. }
            | Self::InvalidDefinition { id, .. }
            | Self::Duplicate { id } => id,
        }
    }

    /// The human-readable rejection reason.
    pub fn reason(&self) -> String {
        match self {
            Self::InvalidId { reason, .. }
            | Self::InvalidConfig { reason, .. }
            | Self::InvalidDefinition { reason, .. } => reason.clone(),
            Self::Duplicate { id } => format!("duplicate capability id {id:?}"),
        }
    }

    /// Whether the error is a duplicate/collision rejection.
    pub fn is_duplicate(&self) -> bool {
        matches!(self, Self::Duplicate { .. })
    }
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { id, reason } => {
                write!(f, "invalid capability id {id:?}: {reason}")
            }
            Self::InvalidConfig { id, reason } => {
                write!(f, "invalid capability config for {id:?}: {reason}")
            }
            Self::InvalidDefinition { id, reason } => {
                write!(f, "invalid capability definition {id:?}: {reason}")
            }
            Self::Duplicate { id } => write!(f, "duplicate capability id {id:?}"),
        }
    }
}

impl std::error::Error for CapabilityError {}
