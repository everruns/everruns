//! Session environment binding errors.

use crate::HarnessRequirementError;

/// Why an Environment could not be fixed to a Session.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionEnvironmentError {
    /// The Session already started and its Environment can no longer change.
    AlreadyStarted,
    /// The Session is already bound to a different Environment.
    AlreadyBound,
    /// The Session is already bound to a different Harness.
    HarnessAlreadyBound,
    /// The selected Environment does not satisfy the bound Harness.
    HarnessRequirement(HarnessRequirementError),
    /// Canonical workspace-backend conflict error name.
    ///
    /// Environment binding continues to emit
    /// [`ProviderConflict`](Self::ProviderConflict) during its deprecation
    /// window.
    BackendConflict,
    /// Compatibility variant emitted during its deprecation window.
    #[deprecated(note = "use BackendConflict")]
    ProviderConflict,
    /// The recorded Environment or workspace head cannot be reopened.
    Unavailable,
    /// The workspace backend rejected the requested operation.
    Workspace(everruns_host::WorkspaceError),
}

impl From<everruns_host::EnvironmentBindingError> for SessionEnvironmentError {
    fn from(error: everruns_host::EnvironmentBindingError) -> Self {
        match error {
            everruns_host::EnvironmentBindingError::Conflict => Self::AlreadyBound,
            everruns_host::EnvironmentBindingError::Unavailable
            | everruns_host::EnvironmentBindingError::Corrupt => Self::Unavailable,
            _ => Self::Unavailable,
        }
    }
}

impl std::fmt::Display for SessionEnvironmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyStarted => formatter.write_str("session execution already started"),
            Self::AlreadyBound => formatter.write_str("session is already bound to another head"),
            Self::HarnessAlreadyBound => {
                formatter.write_str("session is already bound to another harness")
            }
            Self::HarnessRequirement(error) => {
                write!(formatter, "harness requirement failed: {error}")
            }
            Self::BackendConflict => {
                formatter.write_str("another workspace backend uses the same backend id")
            }
            #[allow(deprecated)]
            Self::ProviderConflict => {
                formatter.write_str("another workspace backend uses the same backend id")
            }
            Self::Unavailable => formatter.write_str("environment binding store is unavailable"),
            Self::Workspace(error) => write!(formatter, "workspace selection failed: {error}"),
        }
    }
}

impl std::error::Error for SessionEnvironmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HarnessRequirement(error) => Some(error),
            Self::Workspace(error) => Some(error),
            _ => None,
        }
    }
}

impl From<HarnessRequirementError> for SessionEnvironmentError {
    fn from(error: HarnessRequirementError) -> Self {
        Self::HarnessRequirement(error)
    }
}
