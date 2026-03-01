// Structured audit logging for security-relevant events (TM-OBS-007)
//
// Design: fire-and-forget via tokio::spawn. Audit log failures must never
// block or fail authentication operations. All writes go to the audit_logs
// table via StorageBackend.
//
// Event type convention: domain.action.outcome
//   auth.login.success, auth.login.failure
//   auth.register.success
//   auth.token_refresh.success, auth.token_refresh.failure
//   auth.api_key.created, auth.api_key.deleted
//   auth.oauth.success, auth.oauth.failure

use crate::storage::StorageBackend;
use crate::storage::models::CreateAuditLogRow;
use axum::http::HeaderMap;
use std::sync::Arc;
use uuid::Uuid;

/// Extract client IP from request headers (X-Forwarded-For > X-Real-IP > unknown).
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(forwarded) = headers.get("x-forwarded-for")
        && let Ok(val) = forwarded.to_str()
        && let Some(first) = val.split(',').next()
    {
        let trimmed = first.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(real_ip) = headers.get("x-real-ip")
        && let Ok(val) = real_ip.to_str()
    {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Emit an audit log entry. Non-blocking: spawns a background task.
///
/// Failures are logged at warn level but never propagated.
pub fn emit(
    db: Arc<StorageBackend>,
    org_id: i64,
    actor_id: Option<Uuid>,
    event_type: &str,
    ip_address: Option<String>,
    metadata: serde_json::Value,
) {
    let event_type = event_type.to_string();
    tokio::spawn(async move {
        if let Err(e) = db
            .create_audit_log(CreateAuditLogRow {
                org_id,
                actor_id,
                event_type: event_type.clone(),
                ip_address,
                metadata,
            })
            .await
        {
            tracing::warn!(event_type = %event_type, error = %e, "Failed to write audit log");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_ip_from_x_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&headers), Some("1.2.3.4".to_string()));
    }

    #[test]
    fn test_client_ip_from_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "5.6.7.8".parse().unwrap());
        assert_eq!(client_ip(&headers), Some("5.6.7.8".to_string()));
    }

    #[test]
    fn test_client_ip_prefers_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.1.1.1".parse().unwrap());
        headers.insert("x-real-ip", "2.2.2.2".parse().unwrap());
        assert_eq!(client_ip(&headers), Some("1.1.1.1".to_string()));
    }

    #[test]
    fn test_client_ip_none_when_empty() {
        let headers = HeaderMap::new();
        assert_eq!(client_ip(&headers), None);
    }
}
