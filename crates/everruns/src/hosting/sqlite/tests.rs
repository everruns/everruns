use super::*;

#[test]
fn startup_discovery_uses_indexes_and_selects_only_due_unowned_work_once() {
    let dir = tempfile::tempdir().unwrap();
    let hosting = SqliteHosting::open(dir.path().join("authority.db")).unwrap();
    let db = hosting.db.lock().unwrap();
    for (name, runnable, wake_at, expires, suspended) in [
        ("idle", false, None, 0, false),
        ("runnable", true, None, 0, false),
        ("due", false, Some(99), 0, false),
        ("both", true, Some(99), 0, false),
        ("future", false, Some(101), 0, false),
        ("owned", true, Some(99), 101, false),
        ("suspended", true, Some(99), 0, true),
    ] {
        db.execute(
            "INSERT INTO framework_authority VALUES (?,?)",
            params![name, json!({"runnable":runnable,"wake_at":wake_at,"expires":expires,"suspended":suspended}).to_string()],
        ).unwrap();
    }
    let mut query = db.prepare(DISCOVER_RUNNABLE).unwrap();
    let found: Vec<String> = query
        .query_map([100], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(found, ["both", "due", "runnable"]);

    let mut plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {DISCOVER_RUNNABLE}"))
        .unwrap();
    let plan = plan
        .query_map([100], |row| row.get::<_, String>(3))
        .unwrap()
        .map(|row| row.unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plan.contains("framework_runnable"), "{plan}");
    assert!(plan.contains("framework_timers"), "{plan}");
    assert!(!plan.contains("SCAN framework_authority"), "{plan}");
}
