use crate::llm_error::LlmErrorKind;

/// Structured provider error emitted inside an accepted response stream.
///
/// Providers should preserve the wire error code and HTTP status when they are
/// available. Runtime retry classification uses those fields before falling
/// back to the human-readable message for legacy drivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmStreamError {
    /// Stable machine-readable provider error code, when supplied.
    pub code: Option<String>,
    /// HTTP status associated with the stream error, when supplied.
    pub status: Option<u16>,
    /// Human-readable diagnostic text.
    pub message: String,
}

impl LlmStreamError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            status: None,
            message: message.into(),
        }
    }

    /// Build a stream error while preserving provider-supplied structure.
    pub fn provider(
        code: Option<impl Into<String>>,
        status: Option<u16>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.map(Into::into),
            status,
            message: message.into(),
        }
    }

    /// Map the preserved structure to Everruns' semantic provider error kind.
    pub fn kind(&self) -> LlmErrorKind {
        if let Some(code) = self.code.as_deref()
            && let Some(kind) = LlmErrorKind::from_provider_code(code)
        {
            return kind;
        }
        if let Some(status) = self.status {
            return LlmErrorKind::from_provider_status(status, &self.message);
        }
        LlmErrorKind::from_error_text(&self.message)
    }
}

impl std::error::Error for LlmStreamError {}

impl std::fmt::Display for LlmStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.code, self.status) {
            (Some(code), Some(status)) => write!(f, "{code} ({status}): {}", self.message),
            (Some(code), None) => write!(f, "{code}: {}", self.message),
            (None, Some(status)) => write!(f, "({status}): {}", self.message),
            (None, None) => f.write_str(&self.message),
        }
    }
}

impl From<String> for LlmStreamError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for LlmStreamError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}
