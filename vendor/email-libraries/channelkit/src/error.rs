use std::time::Duration;

/// Failures are actionable and never contain credentials or raw provider bodies.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("webhook authentication failed")]
    Authentication,
    #[error("event or operation belongs to a different installation")]
    Scope,
    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
    #[error("capacity exceeded: {0}")]
    Capacity(&'static str),
    #[error("provider rejected operation: {code}")]
    Rejected { code: String },
    #[error("provider rate limited operation; retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },
    #[error("transport request failed")]
    Transport,
    #[error("provider acceptance is unknown; do not blindly repeat the send")]
    DeliveryUnknown,
    #[error("partial delivery: {source}")]
    PartialDelivery {
        confirmed: Vec<crate::MessageRef>,
        #[source]
        source: Box<Error>,
    },
    #[error("{0}")]
    Observed(#[source] std::sync::Arc<Error>),
    #[error("operation was cancelled")]
    Cancelled,
    #[error("stream deadline exceeded")]
    DeadlineExceeded,
    #[error("provider reply window has closed or its reply context is unavailable")]
    ReplyWindowClosed,
    #[error("malformed provider response")]
    InvalidResponse,
    #[error("handler failed: {0}")]
    Handler(#[source] Box<dyn std::error::Error + Send + Sync>),
}
pub type Result<T> = std::result::Result<T, Error>;
