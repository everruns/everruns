//! Public-surface lifecycle hook contract.
//!
//! This file imports only `everruns::prelude::*`, proving applications can
//! register typed handlers, handle failures, and preserve cancellation without
//! constructing persisted hook records or importing internal crates.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use everruns::prelude::*;

#[test]
fn fixture_uses_only_the_framework_facade() {
    let source = include_str!("lifecycle_hooks.rs");
    assert!(!source.contains(concat!("everruns_", "core")));
    assert!(!source.contains(concat!("everruns_", "runtime")));
}

#[tokio::test]
async fn agent_turn_and_completion_hooks_are_ordered_and_scoped() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let agent_one = seen.clone();
    let agent_two = seen.clone();
    let turn = seen.clone();
    let completion = seen.clone();
    let agent = Agent::builder()
        .name("hooked-agent")
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .on_agent_start(move |context| {
            let agent_one = agent_one.clone();
            async move {
                assert_eq!(context.agent_name, "hooked-agent");
                assert!(!context.session_id.to_string().is_empty());
                agent_one.lock().unwrap().push("agent-1");
            }
        })
        .on_agent_start(move |_context| {
            let agent_two = agent_two.clone();
            async move { agent_two.lock().unwrap().push("agent-2") }
        })
        .on_turn_start(move |context| {
            let turn = turn.clone();
            async move {
                assert_eq!(context.input.role, MessageRole::User);
                turn.lock().unwrap().push("turn");
            }
        })
        .on_completion(move |context| {
            let completion = completion.clone();
            async move {
                assert!(context.turn.success);
                completion.lock().unwrap().push("completion");
            }
        })
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    session.run("one").await.expect("first turn");
    session.run("two").await.expect("second turn");

    assert_eq!(
        *seen.lock().unwrap(),
        [
            "agent-1",
            "agent-2",
            "turn",
            "completion",
            "turn",
            "completion",
        ]
    );
}

#[tokio::test]
async fn agent_start_runs_once_for_each_session() {
    let starts = Arc::new(AtomicUsize::new(0));
    let starts_in_hook = starts.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .on_agent_start(move |_context| {
            let starts_in_hook = starts_in_hook.clone();
            async move {
                starts_in_hook.fetch_add(1, Ordering::SeqCst);
            }
        })
        .build()
        .expect("valid agent");

    let first = InMemoryEngine::new().create(agent.clone());
    first.run("one").await.expect("first session starts");
    first.run("two").await.expect("first session stays started");
    InMemoryEngine::new()
        .create(agent)
        .run("three")
        .await
        .expect("second session starts independently");

    assert_eq!(starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn inspection_does_not_run_lifecycle_hooks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent_call = calls.clone();
    let turn_call = calls.clone();
    let completion_call = calls.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .on_agent_start(move |_context| {
            let agent_call = agent_call.clone();
            async move {
                agent_call.fetch_add(1, Ordering::SeqCst);
            }
        })
        .on_turn_start(move |_context| {
            let turn_call = turn_call.clone();
            async move {
                turn_call.fetch_add(1, Ordering::SeqCst);
            }
        })
        .on_completion(move |_context| {
            let completion_call = completion_call.clone();
            async move {
                completion_call.fetch_add(1, Ordering::SeqCst);
            }
        })
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    session.inspect().await.expect("context can be inspected");
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    session.run("hello").await.expect("turn runs");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn pre_effect_error_is_typed_and_agent_start_retries() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_in_hook = attempts.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .on_agent_start(move |_context| {
            let attempts_in_hook = attempts_in_hook.clone();
            async move {
                if attempts_in_hook.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("dependency unavailable")
                } else {
                    Ok(())
                }
            }
        })
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let error = session.run("first").await.expect_err("start hook fails");
    let RunError::Hook(failure) = error else {
        panic!("expected typed hook failure")
    };
    assert_eq!(failure.point, HookPoint::AgentStart);
    assert_eq!(failure.handler_index, 0);
    assert_eq!(failure.message, "dependency unavailable");

    let turn = session.run("retry").await.expect("start chain retries");
    assert!(turn.success);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn turn_start_error_prevents_the_turn_without_restarting_the_agent() {
    let starts = Arc::new(AtomicUsize::new(0));
    let starts_in_hook = starts.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_in_hook = attempts.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .on_agent_start(move |_context| {
            let starts_in_hook = starts_in_hook.clone();
            async move {
                starts_in_hook.fetch_add(1, Ordering::SeqCst);
            }
        })
        .on_turn_start(move |_context| {
            let attempts_in_hook = attempts_in_hook.clone();
            async move {
                if attempts_in_hook.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("input rejected")
                } else {
                    Ok(())
                }
            }
        })
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let error = session.run("first").await.expect_err("turn hook fails");
    let RunError::Hook(failure) = error else {
        panic!("expected typed hook failure")
    };
    assert_eq!(failure.point, HookPoint::TurnStart);

    session.run("retry").await.expect("turn hook retries");
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn completion_errors_are_isolated_and_visible() {
    let later = Arc::new(AtomicUsize::new(0));
    let later_in_hook = later.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .on_completion(|_context| async move { Err::<(), _>("audit unavailable") })
        .on_completion(move |_context| {
            let later_in_hook = later_in_hook.clone();
            async move {
                later_in_hook.fetch_add(1, Ordering::SeqCst);
            }
        })
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent.clone())
        .run("hello")
        .await
        .expect("turn succeeds");

    assert!(turn.success);
    assert_eq!(later.load(Ordering::SeqCst), 1);
    assert_eq!(turn.hook_failures.len(), 1);
    assert_eq!(turn.hook_failures[0].point, HookPoint::Completion);
    assert_eq!(turn.hook_failures[0].message, "audit unavailable");
}

#[tokio::test]
async fn pre_cancelled_run_skips_every_hook() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_in_hook = calls.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("unreachable"))
        .on_agent_start(move |_context| {
            let calls_in_hook = calls_in_hook.clone();
            async move {
                calls_in_hook.fetch_add(1, Ordering::SeqCst);
            }
        })
        .build()
        .expect("valid agent");
    let token = CancellationToken::new();
    token.cancel();

    let turn = InMemoryEngine::new()
        .create(agent)
        .run_with("hello", RunOptions::new().cancel_token(token))
        .await
        .expect("cancelled run resolves");

    assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
