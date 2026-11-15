//! Reaching a sandbox's guest through the Kubernetes API server's pod proxy
//! (`/api/v1/namespaces/<ns>/pods/<pod>:<port>/proxy/...`), for servers that
//! run outside the cluster. Needs `pods/proxy` in the server's Role.

use async_trait::async_trait;

use crate::sandbox_guest::client::{GuestRequest, GuestResponse, GuestTransport};

pub struct ApiProxyTransport {
    pub client: kube::Client,
    pub namespace: String,
    pub pod: String,
    pub port: u16,
}

#[async_trait]
impl GuestTransport for ApiProxyTransport {
    async fn send(&self, request: GuestRequest) -> Result<GuestResponse, String> {
        let uri = format!(
            "/api/v1/namespaces/{}/pods/{}:{}/proxy{}",
            self.namespace, self.pod, self.port, request.path_and_query
        );
        let mut builder = http::Request::builder()
            .method(request.method.as_str())
            .uri(uri);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let http_request = builder
            .body(kube::client::Body::from(request.body))
            .map_err(|err| err.to_string())?;
        let call = async {
            let response = self
                .client
                .send(http_request)
                .await
                .map_err(|err| format!("Kubernetes API proxy: {err}"))?;
            let status = response.status().as_u16();
            let body = response
                .into_body()
                .collect_bytes()
                .await
                .map_err(|err| format!("Kubernetes API proxy answer cut off: {err}"))?;
            Ok(GuestResponse {
                status,
                body: body.to_vec(),
            })
        };
        tokio::time::timeout(request.timeout, call)
            .await
            .map_err(|_| "Kubernetes API proxy timed out".to_string())?
    }
}
