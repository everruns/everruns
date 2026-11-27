//! Channelkit adapts provider wire protocols; Everruns owns admission and recovery.
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::runtime::{
    channel::{DeliveryReceiptSink, DeliveryResult},
    egress::{EgressRequest, EgressRequestKind, EgressService},
};
use futures::StreamExt;

/// Route every provider HTTP hop through the host's scoped egress service.
/// Channelkit performs provider destination validation and manual redirects.
pub struct EgressTransport {
    egress: Arc<dyn EgressService>,
}

impl EgressTransport {
    pub fn new(egress: Arc<dyn EgressService>) -> Self {
        Self { egress }
    }

    async fn send(
        &self,
        request: channelkit::http::Request,
    ) -> channelkit::Result<channelkit::http::Response> {
        let mut outbound =
            EgressRequest::new(request.method, request.url, EgressRequestKind::Integration)
                .require_dns_pinning()
                .timeout_ms(30_000)
                .body(request.body);
        outbound.headers = request.headers;
        let mut response = self
            .egress
            .send_stream(outbound)
            .await
            .map_err(|_| channelkit::Error::Transport)?;
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            let chunk = chunk.map_err(|_| channelkit::Error::Transport)?;
            if body.len().saturating_add(chunk.len()) > request.max_response_bytes {
                return Err(channelkit::Error::Capacity("HTTP response"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(channelkit::http::Response {
            status: response.status,
            headers: response
                .headers
                .into_iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), value))
                .collect(),
            body,
        })
    }
}

#[async_trait]
impl channelkit::http::Transport for EgressTransport {
    async fn execute(
        &self,
        request: channelkit::http::Request,
    ) -> channelkit::Result<channelkit::http::Response> {
        tokio::time::timeout(std::time::Duration::from_secs(30), self.send(request))
            .await
            .map_err(|_| channelkit::Error::Transport)?
    }
}

/// Provider bridges preserve their existing stored reference encoding. The
/// encoding contains identifiers, never response handles, URLs or credentials.
pub type ReferenceEncoder = fn(&channelkit::MessageRef) -> String;

/// Confirmed split parts reach the host's committed intent before the next send.
pub struct ReceiptObserver<'a> {
    pub sink: &'a dyn DeliveryReceiptSink,
    pub target: &'a channelkit::ConversationKey,
    pub encode: ReferenceEncoder,
}

#[async_trait]
impl channelkit::ReceiptObserver for ReceiptObserver<'_> {
    async fn confirmed(&self, reference: &channelkit::MessageRef) -> channelkit::Result<()> {
        if &reference.conversation != self.target {
            return Err(channelkit::Error::Scope);
        }
        self.sink
            .confirmed(&(self.encode)(reference))
            .await
            .map_err(|_| channelkit::Error::DeliveryUnknown)
    }
}

/// Preserve acceptance and retry advice. A provider's rejection classifier is
/// installation-independent policy; transport ambiguity is never converted to a retry.
pub fn delivery_outcome(
    error: &channelkit::Error,
    permanent: fn(&str) -> bool,
    encode: ReferenceEncoder,
) -> DeliveryResult {
    use channelkit::Error;
    match error {
        Error::Observed(error) => delivery_outcome(error, permanent, encode),
        Error::PartialDelivery { confirmed, source } => DeliveryResult::Partial {
            confirmed: confirmed.iter().map(encode).collect(),
            cause: Box::new(delivery_outcome(source, permanent, encode)),
        },
        Error::RateLimited { retry_after } => DeliveryResult::RateLimited {
            retry_after: *retry_after,
        },
        Error::DeliveryUnknown => DeliveryResult::Unknown {
            confirmed: Vec::new(),
        },
        Error::Rejected { code } if !permanent(code) => {
            DeliveryResult::TransientError(format!("provider rejected operation: {code}"))
        }
        Error::Transport => DeliveryResult::TransientError("provider transport unavailable".into()),
        Error::InvalidResponse => {
            DeliveryResult::TransientError("invalid provider response".into())
        }
        Error::Handler(_) => DeliveryResult::Unknown {
            confirmed: Vec::new(),
        },
        _ => DeliveryResult::PermanentError(error.to_string()),
    }
}

#[cfg(test)]
#[path = "channelkit_bridge_tests.rs"]
mod tests;
