// Structured audit logging for security-relevant events (TM-OBS-007, EVE-226)
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

use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::storage::StorageBackend;
use crate::storage::models::CreateAuditLogRow;
use axum::extract::{ConnectInfo, Extension};
use axum::http::HeaderMap;
use everruns_core::{AuditEvent, AuditLogger};
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

/// Extract the audit client IP using the same trusted-proxy contract as auth
/// rate limiting: forwarding headers count only from trusted peers.
pub fn client_ip(peer: Option<SocketAddr>, headers: &HeaderMap) -> Option<String> {
    Some(extract_client_ip_from_parts(peer, headers).to_string())
}

/// Axum-handler adapter for `client_ip`.
pub fn client_ip_from_connect_info(
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: &HeaderMap,
) -> Option<String> {
    let peer = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    client_ip(peer, headers)
}

/// Emit a legacy audit log entry (event_type string format). Non-blocking.
///
/// For new code, prefer `emit_event()` with typed `AuditEvent`.
pub fn emit(
    db: Arc<StorageBackend>,
    org_id: i64,
    actor_id: Option<Uuid>,
    event_type: &str,
    ip_address: Option<String>,
    metadata: serde_json::Value,
) {
    let event_type = event_type.to_string();
    // Infer domain from event_type prefix
    let domain = if event_type.starts_with("agent.") {
        "agent"
    } else {
        "management"
    };
    let domain_str = domain.to_string();
    let action = event_type.clone();
    tokio::spawn(async move {
        if let Err(e) = db
            .create_audit_log(CreateAuditLogRow {
                org_id,
                actor_id,
                event_type: event_type.clone(),
                ip_address,
                metadata,
                domain: domain_str,
                action,
                target_type: None,
                target_id: None,
            })
            .await
        {
            tracing::warn!(event_type = %event_type, error = %e, "Failed to write audit log");
        }
    });
}

/// Emit a typed audit event. Non-blocking (fire-and-forget).
pub fn emit_event(db: Arc<StorageBackend>, event: AuditEvent) {
    tokio::spawn(async move {
        if let Err(e) = db
            .create_audit_log(CreateAuditLogRow {
                org_id: event.org_id,
                actor_id: event.actor_user_id,
                event_type: event.action.as_str().to_string(),
                ip_address: event.ip_address,
                metadata: event.details,
                domain: event.domain.as_str().to_string(),
                action: event.action.as_str().to_string(),
                target_type: event.target.as_ref().map(|t| t.target_type.clone()),
                target_id: event.target.as_ref().map(|t| t.target_id.clone()),
            })
            .await
        {
            tracing::warn!(error = %e, "Failed to write audit log");
        }
    });
}

/// `AuditLogger` implementation backed by `StorageBackend`.
///
/// Used by the `#[audit]` macro on service methods.
#[derive(Clone)]
pub struct StorageAuditLogger {
    db: Arc<StorageBackend>,
}

impl StorageAuditLogger {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl AuditLogger for StorageAuditLogger {
    async fn log_event(&self, event: AuditEvent) -> anyhow::Result<()> {
        self.db
            .create_audit_log(CreateAuditLogRow {
                org_id: event.org_id,
                actor_id: event.actor_user_id,
                event_type: event.action.as_str().to_string(),
                ip_address: event.ip_address,
                metadata: event.details,
                domain: event.domain.as_str().to_string(),
                action: event.action.as_str().to_string(),
                target_type: event.target.as_ref().map(|t| t.target_type.clone()),
                target_id: event.target.as_ref().map(|t| t.target_id.clone()),
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    #[test]
    fn test_client_ip_uses_rightmost_forwarded_hop_from_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 70.41.3.18".parse().unwrap());
        let peer: SocketAddr = "127.0.0.1:443".parse().unwrap();
        assert_eq!(
            client_ip(Some(peer), &headers),
            Some("70.41.3.18".to_string())
        );
    }

    #[test]
    fn test_client_ip_ignores_spoofed_forwarded_for_from_untrusted_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "8.8.8.8".parse().unwrap());
        let peer: SocketAddr = "198.51.100.10:443".parse().unwrap();
        assert_eq!(
            client_ip(Some(peer), &headers),
            Some("198.51.100.10".to_string())
        );
    }

    #[test]
    fn test_client_ip_from_x_real_ip_from_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "5.6.7.8".parse().unwrap());
        let peer: SocketAddr = "127.0.0.1:443".parse().unwrap();
        assert_eq!(client_ip(Some(peer), &headers), Some("5.6.7.8".to_string()));
    }

    #[test]
    fn test_client_ip_prefers_forwarded_for_from_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.1.1.1".parse().unwrap());
        headers.insert("x-real-ip", "2.2.2.2".parse().unwrap());
        let peer: SocketAddr = "127.0.0.1:443".parse().unwrap();
        assert_eq!(client_ip(Some(peer), &headers), Some("1.1.1.1".to_string()));
    }

    #[test]
    fn test_client_ip_falls_back_to_loopback_without_peer() {
        let headers = HeaderMap::new();
        assert_eq!(client_ip(None, &headers), Some("127.0.0.1".to_string()));
    }
}
