use super::braintrust_delivery::DeliveryAttempt;
use crate::{EgressRequest, EgressRequestKind, EgressService};
use std::sync::Arc;

pub(super) async fn send(
    egress: &Arc<dyn EgressService>,
    url: &str,
    key: &str,
    payload: &impl serde::Serialize,
) -> DeliveryAttempt {
    let Ok(body) = serde_json::to_vec(payload) else {
        return DeliveryAttempt::Permanent {
            class: "encoding".into(),
            reason: "Unable to encode trace batch".into(),
        };
    };
    let request = EgressRequest::new("POST", url, EgressRequestKind::Other("tracing".into()))
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .body(body)
        .timeout_ms(10_000)
        .require_dns_pinning();
    match egress.send(request).await {
        Ok(response) if (200..300).contains(&response.status) => DeliveryAttempt::Success,
        Ok(response) if response.status == 429 || response.status >= 500 => {
            DeliveryAttempt::Retryable(format!("HTTP {}", response.status))
        }
        Ok(response) => DeliveryAttempt::Permanent {
            class: format!("HTTP {}", response.status),
            reason: format!("HTTP {}", response.status),
        },
        Err(_) => DeliveryAttempt::Retryable("Trace export transport failed".into()),
    }
}
