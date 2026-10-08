#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Offline proof: the coordinator starts real threads, each thread runs its
//! own turns and finishes its assignment, the finished work wakes the
//! coordinator, and resolving goes through the coordinator.

use std::time::Duration;

use everruns::coordination::{self, ThreadStatus};
use everruns::{Engine, MessageRole};
use everruns_coordinator_agent::{
    LAUNCH_REQUEST, RESOLVE_REQUEST, all_ready, coordinator, replies, simulated_model,
    wait_for_threads,
};

const WAIT: Duration = Duration::from_secs(20);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coordinator_hands_work_to_threads_and_hears_back() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new();
    let session = engine.create(coordinator(simulated_model(), dir.path()).unwrap());

    session.run(LAUNCH_REQUEST).await.unwrap();
    let threads = wait_for_threads(&session, WAIT, all_ready).await.unwrap();

    let mut titles: Vec<_> = threads.iter().map(|t| t.title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Announcement", "Pricing FAQ"]);
    for thread in &threads {
        assert_eq!(thread.status, ThreadStatus::ReadyForReview, "{thread:?}");
        assert_eq!(thread.assignments, 1);
        assert_eq!(thread.checklist.len(), 2);
        assert!(thread.checklist.iter().all(|(_, status)| status == "done"));
        assert!(thread.summary.as_deref().unwrap().starts_with("Three"));

        // A thread is an ordinary session of the same engine: it got the
        // brief as its first message and ran its own turns.
        let thread_session = engine.resume(thread.id).await.unwrap();
        let history = thread_session.history().page().await.unwrap();
        let first = history.iter().next().unwrap();
        assert_eq!(first.role, MessageRole::User);
        assert!(first.text().contains(&format!(
            "New assignment from the coordinator: {}",
            thread.title
        )));
        assert!(
            history
                .iter()
                .any(|m| m.role == MessageRole::Agent && m.text() == "Finishing up.")
        );
    }

    // Threads share the coordinator's workspace.
    let announcement = std::fs::read_to_string(dir.path().join("announcement.md")).unwrap();
    assert!(announcement.contains("Sign up"));
    assert!(dir.path().join("pricing-faq.md").exists());

    // The finished work woke the coordinator without the person saying anything.
    // The updates may land inside the first turn or start new ones; either way
    // the coordinator answers them.
    let deadline = tokio::time::Instant::now() + WAIT;
    while !replies(&session)
        .await
        .unwrap()
        .iter()
        .any(|reply| reply.contains("ready for review"))
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "coordinator was not woken"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let history = session.history().page().await.unwrap();
    let updates = history
        .iter()
        .filter(|m| m.role == MessageRole::User && m.text().contains("finished: succeeded"))
        .count();
    assert_eq!(updates, 2, "one automatic update per finished thread");

    // Resolving goes through the coordinator.
    session.run(RESOLVE_REQUEST).await.unwrap();
    let threads = coordination::threads(&session).await.unwrap();
    let status = |title: &str| threads.iter().find(|t| t.title == title).unwrap().status;
    assert_eq!(status("Announcement"), ThreadStatus::Resolved);
    assert_eq!(status("Pricing FAQ"), ThreadStatus::ReadyForReview);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_without_threads_has_an_empty_board() {
    let dir = tempfile::tempdir().unwrap();
    let session = Engine::new().create(coordinator(simulated_model(), dir.path()).unwrap());
    assert!(coordination::threads(&session).await.unwrap().is_empty());
}
