use super::queries as q;
use super::types::HealthCheckResponse;
use crate::domains::common::*;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct HealthCheck {}

impl Command for HealthCheck {
    type Output = HealthCheckResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "health_check",
            category: "system",
            description: "Health check endpoint.",
            method: "GET",
            path: "/health",
        }
    }

    async fn execute(self, _ctx: &Ctx) -> Result<HealthCheckResponse, CommandError> {
        Ok(HealthCheckResponse {
            status: "ok",
            version: q::version(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<HealthCheck>() }

#[cfg(test)]
mod tests {
    use crate::domains::common::Ctx;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn health_check_dispatch_accepts_empty_object_params() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = Ctx::minimal_for_test(
            Caller {
                org_id: DEFAULT_ORG_ID,
                org_public_id: "org_00000000000000000000000000000001".to_string(),
                user_id: None,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            None,
        );

        let json = crate::domains::common::dispatch("health_check", json!({}), &ctx)
            .await
            .expect("dispatch health_check");
        let response: serde_json::Value =
            serde_json::from_str(&json).expect("deserialize health_check response");
        assert_eq!(response.get("status"), Some(&json!("ok")));
    }
}
