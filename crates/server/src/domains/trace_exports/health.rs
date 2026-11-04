//! One organization health issue summarizes failing enabled destinations.
use crate::storage::StorageBackend;
pub const CODE: &str = "tracing.delivery";
pub async fn refresh(db: &StorageBackend, org: i64) -> anyhow::Result<()> {
    let failing:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tracing_destinations WHERE org_id=$1 AND config->>'enabled'='true' AND last_error IS NOT NULL)")
        .bind(org).fetch_one(db.database().tx_pool()).await?;
    db.observe_org_health_issue(
        org,
        CODE,
        if failing { "open" } else { "resolved" },
        chrono::Utc::now(),
        (
            "Trace delivery recovered",
            "Enabled destinations no longer report delivery failures.",
        ),
    )
    .await
}
