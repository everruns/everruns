//! Recheck revocation at the actual network boundary, including SDK retries.
use super::TraceSecrets;
use crate::storage::{StorageBackend, trace_exports::TracingDestinationRow};
use async_trait::async_trait;
use everruns_core::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressScope, EgressService,
    EgressStreamResponse,
};
use std::sync::Arc;

pub struct DestinationTransport {
    pub db: Arc<StorageBackend>,
    pub inner: Arc<dyn EgressService>,
    pub row: TracingDestinationRow,
    pub scope: EgressScope,
    pub secrets: TraceSecrets,
}
#[async_trait]
impl EgressService for DestinationTransport {
    async fn send(&self, mut request: EgressRequest) -> EgressResult<EgressResponse> {
        let current = self
            .db
            .tracing_export_destination(self.row.org_id, self.row.id)
            .await
            .map_err(|_| EgressError::Transport("Destination lookup failed".into()))?
            .ok_or_else(|| EgressError::invalid("Destination removed"))?;
        let config: super::DestinationConfig = serde_json::from_value(current.config)
            .map_err(|_| EgressError::invalid("Invalid destination configuration"))?;
        if current.revision != self.row.revision || !config.enabled {
            return Err(EgressError::invalid("Destination changed or paused"));
        }
        request.scope = Some(self.scope.clone());
        request.dns_pinning_required = true;
        // No process-wide OTEL headers may cross into customer exports.
        request
            .headers
            .retain(|k, _| k.eq_ignore_ascii_case("content-type"));
        request.headers.extend(self.secrets.headers.clone());
        let result = self.inner.send(request).await;
        let error = match &result {
            Ok(r) if (200..300).contains(&r.status) => {
                if config.provider == super::TraceProvider::Otlp && !otlp_accepted(&r.body) {
                    Some("Receiver rejected spans or returned an invalid OTLP response".into())
                } else {
                    None
                }
            }
            Ok(r) => Some(format!("Receiver returned HTTP {}", r.status)),
            Err(_) => {
                Some("Export failed: endpoint unavailable or blocked by network policy".to_string())
            }
        };
        let _ = self
            .db
            .record_tracing_delivery(
                self.row.org_id,
                self.row.id,
                self.row.revision,
                error.as_deref(),
            )
            .await;
        let _ = super::health::refresh(&self.db, self.row.org_id).await;
        result
    }
    async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::invalid(
            "Streaming trace exports are not supported",
        ))
    }
}

// OTLP partial success is HTTP 200, but rejected spans must not be reported as accepted.
#[derive(prost::Message)]
struct ExportResponse {
    #[prost(message, optional, tag = "1")]
    partial_success: Option<PartialSuccess>,
}
#[derive(prost::Message)]
struct PartialSuccess {
    #[prost(int64, tag = "1")]
    rejected_spans: i64,
    #[prost(string, tag = "2")]
    error_message: String,
}
fn otlp_accepted(body: &[u8]) -> bool {
    use prost::Message;
    ExportResponse::decode(body)
        .is_ok_and(|r| r.partial_success.is_none_or(|p| p.rejected_spans == 0))
}
#[cfg(test)]
#[test]
fn customer_tracing_otlp_partial_rejection_is_not_success() {
    use prost::Message;
    assert!(otlp_accepted(&[]));
    assert!(!otlp_accepted(b"invalid"));
    assert!(!otlp_accepted(
        &ExportResponse {
            partial_success: Some(PartialSuccess {
                rejected_spans: 1,
                error_message: "private receiver details".into()
            })
        }
        .encode_to_vec()
    ));
}
