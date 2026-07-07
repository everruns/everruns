/// Errors that can occur when loading configuration from environment variables.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required environment variable was not set.
    #[error("required env var {var} is not set")]
    Missing { var: String },

    /// An environment variable had a value that could not be parsed.
    #[error("env var {var} has invalid value \"{value}\": {reason}")]
    Invalid {
        var: String,
        value: String,
        reason: String,
    },
}
