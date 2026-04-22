use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HealthCheckResponse {
    pub status: &'static str,
    pub version: &'static str,
}
