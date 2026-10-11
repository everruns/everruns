//! Outbound requests for search drivers, always through the egress boundary.

use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::egress::{
    EgressError, EgressRequest, EgressRequestKind, EgressResponse, EgressResult, EgressService,
    EgressStreamResponse,
};
use everruns_contracts::runtime::web_search::{
    SearchContext, SearchExchange, SearchFailure, SearchFailureKind,
};

const TIMEOUT: Duration = Duration::from_secs(12);

/// A neutral, current desktop browser string. Search pages serve their plain
/// HTML layout to it; an unknown agent string gets a bot check sooner.
pub(super) const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0";

/// GET `url` and record the exchange. Headers carry credentials, so they are
/// never recorded; `url` must not.
pub(super) async fn get(
    context: &SearchContext,
    url: &str,
    headers: &[(&str, &str)],
) -> (SearchExchange, Result<EgressResponse, SearchFailure>) {
    let mut request = EgressRequest::new("GET", url, EgressRequestKind::Integration)
        .network_access(context.network_access.clone())
        .timeout_ms(TIMEOUT.as_millis() as u64)
        .require_dns_pinning();
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let exchange = SearchExchange::get(url);
    match context.egress.send(request).await {
        Ok(response) => (
            exchange.with_body(response.status, &response.body),
            Ok(response),
        ),
        Err(error) => {
            let kind = match error {
                EgressError::NetworkAccessDenied { .. } => SearchFailureKind::NetworkDenied,
                _ => SearchFailureKind::Unavailable,
            };
            let message = error.to_string();
            (
                exchange.with_error(message.clone()),
                Err(SearchFailure::new(kind, message)),
            )
        }
    }
}

/// Map an HTTP status to a failure, or `None` for success.
pub(super) fn status_failure(label: &str, status: u16) -> Option<SearchFailure> {
    let (kind, what) = match status {
        200..=299 => return None,
        401 | 403 => (SearchFailureKind::Unauthorized, "rejected the key"),
        429 => (SearchFailureKind::RateLimited, "was rate limited"),
        _ => (SearchFailureKind::Unavailable, "returned an error"),
    };
    Some(SearchFailure::new(
        kind,
        format!("{label} {what} ({status})"),
    ))
}

/// Egress for hosts without an egress service (embedded tests and tools run
/// outside a session). Plain HTTPS with no policy, like the other direct
/// integration clients.
pub(super) struct DirectEgress {
    client: reqwest::Client,
}

impl DirectEgress {
    pub(super) fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl EgressService for DirectEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| EgressError::invalid(error.to_string()))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(timeout) = request.timeout_ms {
            builder = builder.timeout(Duration::from_millis(timeout));
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_string()))
            })
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?
            .to_vec();
        Ok(EgressResponse {
            status,
            headers,
            body,
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::invalid("search drivers do not stream"))
    }

    fn name(&self) -> &'static str {
        "web_search::DirectEgress"
    }
}

/// Percent-encode a query parameter value.
pub(super) fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Percent-decode a query parameter value (`+` is a space).
pub(super) fn decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let raw = "tokio 1.50 & \"release notes\"/ü";
        assert_eq!(decode(&encode(raw)), raw);
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("a%2"), "a%2");
    }

    #[test]
    fn statuses_map_to_failure_kinds() {
        assert!(status_failure("Brave", 200).is_none());
        assert_eq!(
            status_failure("Brave", 429).unwrap().kind,
            SearchFailureKind::RateLimited
        );
        assert_eq!(
            status_failure("Brave", 401).unwrap().kind,
            SearchFailureKind::Unauthorized
        );
    }
}
