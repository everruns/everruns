//! Session environment binding and negotiation failures.

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
    /// The bound Environment does not provide a compute capability required by the Harness.
    MissingHarnessCapability {
        /// The missing [`ComputeCapabilities`](everruns_host::ComputeCapabilities) field.
        capability: &'static str,
    },
    /// The bound Environment provides less containment than the Harness requires.
    InsufficientHarnessContainment {
        /// The minimum containment declared by the Harness.
        required: everruns_host::ContainmentLevel,
        /// The containment provided by the Environment.
        available: everruns_host::ContainmentLevel,
    },
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
            Self::MissingHarnessCapability { capability } => write!(
                formatter,
                "harness requires compute capability `{capability}`, but the environment does not provide it"
            ),
            Self::InsufficientHarnessContainment {
                required,
                available,
            } => write!(
                formatter,
                "harness requires `{required}` containment, but the environment provides `{available}`"
            ),
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

impl std::error::Error for SessionEnvironmentError {}
