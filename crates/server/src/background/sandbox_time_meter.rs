// Sandbox time meter: turns each closed `ready` span of a sandbox into a usage
// record with its cost, so sandbox running time shows on the org's Costs page
// and counts against usage policies priced in dollars.
//
// Decision: only closed spans are metered. A running sandbox is charged when
// it stops (or is paused), never estimated while it runs, so each span is
// recorded exactly once (`BudgetService::meter_sandbox_spans` skips spans that
// already have a usage record). Without a price-list row for the provider the
// usage is recorded as unpriced: sandboxes have no list price.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, warn};

use crate::background::cluster_jobs::ClusterJob;
use crate::domains::budgets::BudgetService;

/// Durable schedule name of the sandbox time meter.
pub const SANDBOX_TIME_METER_SCHEDULE: &str = "sandbox-time-meter";
/// Activity type the sandbox time meter schedule enqueues.
pub const SANDBOX_TIME_METER_ACTIVITY: &str = "sandbox_time_meter";

const METER_PERIOD: Duration = Duration::from_secs(300);
/// Most spans metered per run; the next run picks up the rest.
const METER_BATCH: i64 = 500;

/// The meter as a cluster-once job, every five minutes.
pub fn meter_job(budgets: Arc<BudgetService>) -> ClusterJob {
    ClusterJob::every(
        SANDBOX_TIME_METER_SCHEDULE,
        SANDBOX_TIME_METER_ACTIVITY,
        "Records sandbox running time as usage with its cost from the org's price list.",
        METER_PERIOD,
        move || {
            let budgets = budgets.clone();
            Box::pin(async move {
                match budgets.meter_sandbox_spans(METER_BATCH).await {
                    Ok(0) => debug!("no sandbox time to meter"),
                    Ok(count) => debug!(count, "metered sandbox running time"),
                    Err(error) => warn!(%error, "Sandbox time meter failed"),
                }
            })
        },
    )
}
