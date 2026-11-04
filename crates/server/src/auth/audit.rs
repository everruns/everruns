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
use crate::domains::audit_logs::record::{AuditEvent, AuditLogger};
use crate::storage::CreateAuditLogRow;
use crate::storage::StorageBackend;
use axum::extract::{ConnectInfo, Extension};
use axum::http::HeaderMap;
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

fn attribute_mcp(actor: Option<Uuid>, metadata: &mut serde_json::Value) {
    if let Some(client) = super::mcp_attribution::for_user(actor) {
        if !metadata.is_object() {
            *metadata = serde_json::json!({"data": metadata.take()});
        }
        metadata["mcp_client"] = serde_json::json!(client);
    }
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
    mut metadata: serde_json::Value,
) {
    attribute_mcp(actor_id, &mut metadata);
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
pub fn emit_event(db: Arc<StorageBackend>, mut event: AuditEvent) {
    attribute_mcp(event.actor_user_id, &mut event.details);
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
    async fn log_event(&self, mut event: AuditEvent) -> anyhow::Result<()> {
        attribute_mcp(event.actor_user_id, &mut event.details);
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

    fn emit(&self, event: AuditEvent) {
        // Capture the request provenance before the background task boundary.
        emit_event(self.db.clone(), event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    #[tokio::test]
    async fn mcp_audit_captures_the_actor_before_spawning_and_preserves_details() {
        use crate::domains::audit_logs::record::ManagementAction;
        let db = Arc::new(StorageBackend::test_database());
        let logger = StorageAuditLogger::new(db.clone());
        let mut user = crate::auth::AuthUser::anonymous();
        db.create_test_user(user.id).await;
        user.mcp_client = Some(crate::auth::mcp_attribution::McpClientAttribution {
            client_id: "client_a".into(),
            grant_id: Uuid::now_v7(),
            client_name: "Self-declared".into(),
        });
        crate::auth::mcp_attribution::scope(&user, async {
            let event = AuditEvent::management(ManagementAction::AgentCreated, 1, Some(user.id))
                .detail("name", "agent")
                .build();
            logger.log_event(event.clone()).await.unwrap();
            logger.emit(event);
            emit(
                db.clone(),
                1,
                Some(user.id),
                "agent.created",
                None,
                serde_json::json!("legacy"),
            );
        })
        .await;
        let rows = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let rows = db
                    .list_audit_logs(crate::storage::AuditLogQuery {
                        org_id: 1,
                        limit: 10,
                        ..Default::default()
                    })
                    .await
                    .unwrap();
                if rows.len() == 3 {
                    break rows;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for row in &rows {
            assert_eq!(row.metadata["mcp_client"]["client_id"], "client_a");
            assert_eq!(row.actor_id, Some(user.id));
        }
        assert!(rows.iter().any(|r| r.metadata["data"] == "legacy"));
        assert!(rows.iter().any(|r| r.metadata["name"] == "agent"));
    }

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
