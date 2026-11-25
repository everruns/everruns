//! Loopback-only proxy for conformance: authenticate through the real PAT path.
use super::*;
impl TestServer {
    pub async fn serve_mcp_conformance(&self, token: &str) -> String {
        let (listener, base) = Self::bind().await;
        let authorization = axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap();
        let router = self.router.clone().layer(axum::middleware::map_request(
            move |mut request: axum::extract::Request| {
                let authorization = authorization.clone();
                async move {
                    request.headers_mut().insert("authorization", authorization);
                    request
                }
            },
        ));
        Self::serve(listener, router).await;
        base
    }
}
