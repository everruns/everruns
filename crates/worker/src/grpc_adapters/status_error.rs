use super::*;

/// Map a tonic gRPC status to the appropriate AgentLoopError variant.
///
/// Preserves the semantic meaning of gRPC status codes so that callers
/// (e.g. retry logic in the durable engine) can distinguish transient
/// transport errors from permanent domain errors.
pub(crate) fn grpc_status_to_error(status: tonic::Status) -> AgentLoopError {
    let msg = status.message().to_string();
    match status.code() {
        tonic::Code::NotFound => {
            // Map to specific "not found" variants when possible
            if msg.contains("Session") {
                AgentLoopError::store(format!("Session not found: {msg}"))
            } else if msg.contains("Agent") {
                AgentLoopError::store(format!("Agent not found: {msg}"))
            } else if msg.contains("Harness") {
                AgentLoopError::store(format!("Harness not found: {msg}"))
            } else {
                AgentLoopError::store(format!("Not found: {msg}"))
            }
        }
        tonic::Code::InvalidArgument => AgentLoopError::config(format!("Invalid argument: {msg}")),
        tonic::Code::Unavailable => AgentLoopError::store(format!("Service unavailable: {msg}")),
        tonic::Code::ResourceExhausted => {
            let msg_lower = msg.to_ascii_lowercase();
            if msg_lower.contains("message")
                || msg_lower.contains("payload")
                || msg_lower.contains("size")
                || msg_lower.contains("too large")
                || msg_lower.contains("context length")
            {
                AgentLoopError::request_too_large(msg)
            } else {
                AgentLoopError::store(format!("Resource exhausted: {msg}"))
            }
        }
        tonic::Code::Unauthenticated | tonic::Code::PermissionDenied => {
            AgentLoopError::config(format!("Auth error: {msg}"))
        }
        _ => AgentLoopError::store(format!("gRPC error ({}): {msg}", status.code())),
    }
}
