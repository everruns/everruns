//! In-process cron for `dev` and self-hosted `start`.
//!
//! A hosted deploy creates the cron entries from the manifest instead and
//! calls the schedule; this loop is what the same binary does on its own.
//!
//! Decision (actor-based design, step 3): a schedule's last handled occurrence
//! is stored, so a timer outlives the process. On boot, an occurrence that fell
//! due while the host was down runs once, right away; several missed
//! occurrences collapse into that one run. The first boot only records a
//! starting point. Each occurrence is recorded before it runs, so a crash
//! mid-run does not run it again.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::app::ScheduleEntry;
use crate::host::Host;

/// Spawn one task per schedule. Each sleeps until its next fire time, runs,
/// and reports the outcome on stdout.
pub(crate) fn spawn(host: &Arc<Host>) {
    for entry in host.app.inner.schedules.clone() {
        let host = Arc::downgrade(host);
        tokio::spawn(async move {
            let name = entry.registration.name;
            if let Some(due) = host
                .upgrade()
                .and_then(|host| missed(&host, &entry, Utc::now()))
            {
                println!("⏰ schedule {name} missed {due} while stopped; running it now");
                if let Some(host) = host.upgrade() {
                    fire(&host, name, due).await;
                }
            }
            loop {
                let Some(next) = entry.schedule.upcoming(Utc).next() else {
                    return;
                };
                let wait = (next - Utc::now()).to_std().unwrap_or_default();
                tokio::time::sleep(wait).await;
                let Some(host) = host.upgrade() else {
                    return;
                };
                fire(&host, name, next).await;
            }
        });
    }
}

/// The latest occurrence of `entry` that fell due after the last one this host
/// handled and before `now`. With no record yet, records `now` as the starting
/// point and returns `None`.
fn missed(host: &Host, entry: &ScheduleEntry, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let name = entry.registration.name;
    let last = match host.store.schedule_due_at(name) {
        Ok(Some(last)) => last,
        Ok(None) => {
            if let Err(err) = host.store.record_schedule_due(name, now) {
                eprintln!("⏰ schedule {name}: could not record its start: {err:#}");
            }
            return None;
        }
        Err(err) => {
            eprintln!("⏰ schedule {name}: could not read its last run: {err:#}");
            return None;
        }
    };
    last_missed(&entry.schedule, last, now)
}

/// The latest occurrence of `schedule` after `last` and no later than `now`.
fn last_missed(
    schedule: &cron::Schedule,
    last: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    schedule.after(&last).take_while(|due| *due <= now).last()
}

async fn fire(host: &Host, name: &'static str, due: DateTime<Utc>) {
    if let Err(err) = host.store.record_schedule_due(name, due) {
        eprintln!("⏰ schedule {name}: could not record the run: {err:#}");
    }
    println!("⏰ schedule {name} firing");
    match host.run_schedule(name).await {
        Ok(()) => println!("⏰ schedule {name} done"),
        Err(err) => eprintln!("⏰ schedule {name} failed: {err:#}"),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::TimeZone;

    use super::*;
    use crate::store::Store;

    fn hourly() -> cron::Schedule {
        cron::Schedule::from_str("0 0 * * * *").unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 10, hour, minute, 0).unwrap()
    }

    #[test]
    fn several_missed_occurrences_collapse_into_the_latest() {
        assert_eq!(
            last_missed(&hourly(), at(9, 0), at(12, 30)),
            Some(at(12, 0))
        );
    }

    #[test]
    fn nothing_is_missed_before_the_next_occurrence() {
        assert_eq!(last_missed(&hourly(), at(9, 0), at(9, 59)), None);
    }

    #[test]
    fn the_last_handled_occurrence_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.db");
        Store::open(&path)
            .unwrap()
            .record_schedule_due("digest", at(9, 0))
            .unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.schedule_due_at("digest").unwrap(), Some(at(9, 0)));
        assert_eq!(reopened.schedule_due_at("other").unwrap(), None);
    }
}
