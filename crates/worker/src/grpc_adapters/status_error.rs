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
            // The control plane names a deleted session as `Session not
            // found: <id>` (e.g. an event write after the delete, EVE-1235);
            // keep it typed so the turn driver stops the turn quietly.
            if let Some(session_id) = msg
                .strip_prefix("Session not found: ")
                .and_then(|id| id.parse::<SessionId>().ok())
            {
                AgentLoopError::session_not_found(session_id)
            } else if msg.contains("Session") {
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
