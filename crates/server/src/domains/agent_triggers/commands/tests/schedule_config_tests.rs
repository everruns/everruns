//! Schedule trigger config: cron validation and the trigger's timezone.

use super::*;

#[test]
fn validate_schedule_config_accepts_daily_cron() {
    let normalized =
        validate_schedule_config("0 9 * * *", "UTC", "hello").expect("valid daily cron");
    // 5-field input is normalized to 7-field (sec … year).
    assert_eq!(normalized, "0 0 9 * * * *");
}

/// Weekdays of the next seven runs of a normalized (crate-form) cron, from a
/// fixed Monday so the result does not depend on today.
fn weekdays_of(normalized: &str) -> Vec<chrono::Weekday> {
    use chrono::{Datelike, TimeZone};
    let schedule = cron::Schedule::from_str(normalized).expect("crate form parses");
    let monday = chrono::Utc
        .with_ymd_and_hms(2026, 10, 12, 12, 0, 0)
        .unwrap();
    let mut days: Vec<_> = schedule
        .after(&monday)
        .take(7)
        .map(|t| t.weekday())
        .collect();
    days.sort_by_key(|day| day.num_days_from_monday());
    days.dedup();
    days
}

/// Standard cron counts weekdays 0-6 from Sunday; the `cron` crate counts 1-7
/// from Sunday. The documented example `0 9 * * 1-5` fired Sunday-Thursday and
/// `0 18 * * 0` was rejected (EVE-1250).
#[test]
fn validate_schedule_config_reads_five_field_weekdays_from_sunday_zero() {
    use chrono::Weekday::*;
    let weekdays = validate_schedule_config("0 9 * * 1-5", "UTC", "hello").expect("weekday cron");
    assert_eq!(weekdays_of(&weekdays), vec![Mon, Tue, Wed, Thu, Fri]);
    let sunday = validate_schedule_config("0 18 * * 0", "UTC", "hello").expect("Sunday cron");
    assert_eq!(weekdays_of(&sunday), vec![Sun]);
    // Seven fields are the crate's own form and keep its numbering.
    let crate_form = validate_schedule_config("0 0 9 * * 2-6 *", "UTC", "hello").expect("7-field");
    assert_eq!(crate_form, "0 0 9 * * 2-6 *");
    assert_eq!(weekdays_of(&crate_form), vec![Mon, Tue, Wed, Thu, Fri]);
}

#[test]
fn validate_schedule_config_rejects_empty_message() {
    let err = validate_schedule_config("0 9 * * *", "UTC", "   ").unwrap_err();
    assert!(err.message().contains("non-empty message"), "got: {err}");
}

#[test]
fn validate_schedule_config_rejects_too_frequent() {
    // Every minute is below the 300s default minimum interval.
    let err = validate_schedule_config("* * * * *", "UTC", "hi").unwrap_err();
    assert!(err.message().contains("no more than once"), "got: {err}");
}

#[test]
fn validate_schedule_config_rejects_garbage_cron() {
    let err = validate_schedule_config("not a cron", "UTC", "hi").unwrap_err();
    assert!(err.message().to_lowercase().contains("cron"), "got: {err}");
}

#[test]
fn validate_schedule_config_rejects_unknown_timezone() {
    let err = validate_schedule_config("0 9 * * *", "America/Chicgo", "hi").unwrap_err();
    assert!(
        err.message().contains("Invalid IANA timezone"),
        "got: {err}"
    );
    validate_schedule_config("0 9 * * *", "America/Chicago", "hi").expect("valid zone");
    validate_schedule_config("0 9 * * *", "", "hi").expect("empty means UTC");
}

/// Regression: an `America/Chicago` 10:00 trigger got its next run at 10:00
/// UTC because the zone was ignored (prod, 2026-11-08).
#[test]
fn schedule_next_trigger_is_read_in_the_trigger_timezone() {
    use chrono::Timelike;
    let next = crate::domains::agent_channels::invocation::calculate_schedule_next_trigger(
        "0 10 * * *",
        "America/Chicago",
    )
    .expect("valid")
    .expect("has a next run");
    let local = next.with_timezone(&chrono_tz::America::Chicago);
    assert_eq!((local.hour(), local.minute()), (10, 0), "{next}");
    assert_ne!(next.hour(), 10, "{next}");
}
