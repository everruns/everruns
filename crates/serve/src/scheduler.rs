//! In-process cron for `dev` and self-hosted `start`.
//!
//! A hosted deploy creates the cron entries from the manifest instead and
//! calls the schedule; this loop is what the same binary does on its own.

use std::sync::Arc;

use crate::host::Host;

/// Spawn one task per schedule. Each sleeps until its next fire time, runs,
/// and reports the outcome on stdout.
pub(crate) fn spawn(host: &Arc<Host>) {
    for entry in host.app.inner.schedules.clone() {
        let host = Arc::downgrade(host);
        tokio::spawn(async move {
            let name = entry.registration.name;
            loop {
                let Some(next) = entry.schedule.upcoming(chrono::Utc).next() else {
                    return;
                };
                let wait = (next - chrono::Utc::now()).to_std().unwrap_or_default();
                tokio::time::sleep(wait).await;
                let Some(host) = host.upgrade() else {
                    return;
                };
                println!("⏰ schedule {name} firing");
                match host.run_schedule(name).await {
                    Ok(()) => println!("⏰ schedule {name} done"),
                    Err(err) => eprintln!("⏰ schedule {name} failed: {err:#}"),
                }
            }
        });
    }
}
