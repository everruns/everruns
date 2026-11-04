//! Worker registry cases the system doctor reads: what a worker says about
//! itself at registration, its self-check report, and when it began draining.

use super::*;

async fn listed<H: Harness>(h: &H, id: &str) -> WorkerInfo {
    h.store()
        .list_workers(WorkerFilter::default())
        .await
        .unwrap()
        .into_iter()
        .find(|info| info.id == id)
        .expect("registered worker is listed")
}

pub(super) async fn registration_keeps_what_the_worker_reports<H: Harness>(h: H) {
    let id = format!("conformance-worker-{}", Uuid::now_v7().simple());
    let mut info = WorkerInfo::new(id.clone(), [activity_type()]);
    info.hostname = Some("worker-host".to_string());
    info.version = Some("1.2.3".to_string());
    info.metadata = Some(json!({ "protocol_version": 4 }));
    h.store().register_worker(info).await.unwrap();

    let info = listed(&h, &id).await;
    assert_eq!(info.hostname.as_deref(), Some("worker-host"));
    assert_eq!(info.version.as_deref(), Some("1.2.3"));
    assert_eq!(info.protocol_version(), Some(4));
    assert!(info.self_checks().is_none(), "no report before the first");
    assert!(info.draining_since().is_none());
}

pub(super) async fn self_checks_replace_the_previous_report<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    for status in ["problem", "ok"] {
        h.store()
            .record_worker_checks(&w, json!([{ "id": "egress_proxy", "status": status }]))
            .await
            .unwrap();
    }
    let info = listed(&h, &w).await;
    let (checks, at) = info.self_checks().expect("report kept");
    assert_eq!(checks[0]["status"], "ok");
    assert!(at <= chrono::Utc::now());

    // Unknown workers are a no-op, like the other registry updates.
    h.store()
        .record_worker_checks("unknown", json!([]))
        .await
        .unwrap();
}

pub(super) async fn draining_records_when_it_started<H: Harness>(h: H) {
    let ty = activity_type();
    let w = worker(&h, &ty).await;
    h.store().record_worker_checks(&w, json!([])).await.unwrap();

    h.store().drain_worker(&w).await.unwrap();
    let since = listed(&h, &w).await.draining_since().expect("drain time");
    // Draining again keeps the original start.
    h.store().drain_worker(&w).await.unwrap();
    let info = listed(&h, &w).await;
    assert_eq!(info.draining_since(), Some(since));
    assert!(info.self_checks().is_some(), "draining keeps the report");

    h.store().resume_worker(&w).await.unwrap();
    assert!(listed(&h, &w).await.draining_since().is_none());
}
