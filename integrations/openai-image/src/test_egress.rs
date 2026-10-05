//! Test egress transports for the image client (EVE-1174).

use async_trait::async_trait;
use everruns_contracts::runtime::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use everruns_core::host::DirectEgressService;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

/// Real [`DirectEgressService`] (ACL, no redirects) for wiremock servers on
/// loopback. It asserts every request asked for DNS pinning, then clears the
/// flag only for `127.0.0.1` so the local mock is reachable; any other host
/// keeps the flag and goes through `blocked_resolver` answers.
pub(crate) struct LoopbackTestEgress {
    inner: DirectEgressService,
    pub(crate) seen: Mutex<Vec<EgressRequest>>,
}

impl LoopbackTestEgress {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: rebinding_egress("10.0.0.1".parse().unwrap()),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn admit(&self, mut request: EgressRequest) -> EgressRequest {
        assert!(
            request.dns_pinning_required,
            "image requests must require DNS pinning"
        );
        self.seen.lock().unwrap().push(request.clone());
        let loopback = reqwest::Url::parse(&request.url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host == "127.0.0.1"))
            .unwrap_or(false);
        if loopback {
            request.dns_pinning_required = false;
        }
        request
    }
}

#[async_trait]
impl EgressService for LoopbackTestEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        self.inner.send(self.admit(request)).await
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        self.inner.send_stream(self.admit(request)).await
    }
}

/// Real egress whose controlled resolver answers every hostname with
/// `answer`, simulating a public hostname that resolves (or is rebound) to an
/// internal address.
pub(crate) fn rebinding_egress(answer: IpAddr) -> DirectEgressService {
    DirectEgressService::new().with_dns_resolver(move |_host, port| async move {
        Ok(vec![SocketAddr::new(answer, port)])
    })
}

/// Private, loopback, link-local, and metadata answers that must be refused.
pub(crate) const BLOCKED_ANSWERS: &[&str] = &[
    "127.0.0.1",
    "10.0.0.1",
    "192.168.1.10",
    "169.254.169.254",
    "::1",
    "fe80::1",
];
