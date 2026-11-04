//! Retry time budget tests for [`retry_request`](super::retry_request).
//!
//! Split from `llm_retry.rs` to keep that file under the size threshold.
//! Also covers the budget primitives: `reserve_retry_wait`,
//! `next_retry_wait` and `remaining_retry_time`.

use super::*;

#[cfg(feature = "http")]
fn response(status: u16) -> reqwest::Response {
    reqwest::Response::from(
        http::Response::builder()
            .status(status)
            .body(String::new())
            .unwrap(),
    )
}

fn budget_config() -> LlmRetryConfig {
    LlmRetryConfig {
        max_retries: 5,
        initial_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(1),
        backoff_multiplier: 1.0,
        jitter_factor: 0.0,
        max_retry_elapsed: Duration::from_secs(30),
    }
}

#[cfg(feature = "http")]
/// A provider wait longer than the remaining budget falls back to backoff
/// instead of failing with zero retries.
#[tokio::test(start_paused = true)]
async fn oversized_wait_falls_back_to_backoff() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (resp, meta) = retry_request::<_, _, _, _, _, reqwest::Response, reqwest::Error>(
        &budget_config(),
        "TestDriver",
        || {
            let calls = calls.clone();
            async move {
                let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(response(if n == 0 { 429 } else { 200 }))
            }
        },
        |_resp, _attempt, _can_retry| async {
            RetryDecision::Retry {
                wait: Duration::from_secs(45),
                rate_limit_info: Some(RateLimitInfo {
                    retry_after_secs: Some(45),
                    ..Default::default()
                }),
            }
        },
        |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
    )
    .await
    .expect("one retry fits the budget");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        (meta.attempts, meta.total_retry_wait),
        (1, Duration::from_secs(1))
    );
}

#[cfg(feature = "http")]
/// The exhausted error reports time actually spent, not the configured budget.
#[tokio::test(start_paused = true)]
async fn budget_exhausted_error_reports_elapsed_time() {
    let mut config = budget_config();
    // Backoff cannot fit either once 10s of the 30s budget are spent.
    config.initial_backoff = Duration::from_secs(25);
    config.max_backoff = Duration::from_secs(25);
    let waits = std::sync::Mutex::new(vec![Duration::from_secs(25), Duration::from_secs(10)]);
    let err = retry_request::<_, _, _, _, _, reqwest::Response, reqwest::Error>(
        &config,
        "TestDriver",
        || async { Ok(response(503)) },
        |_resp, _attempt, _can_retry| {
            let wait = waits.lock().unwrap().pop().unwrap();
            async move {
                RetryDecision::Retry {
                    wait,
                    rate_limit_info: None,
                }
            }
        },
        |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
    )
    .await
    .expect_err("second wait does not fit the remaining budget");
    assert_eq!(
        err.to_string(),
        "LLM error: TestDriver retry time budget exhausted after 1 retries in 10.0s of a 30.0s budget"
    );
}

#[tokio::test(start_paused = true)]
async fn retry_wait_reservations_enforce_strict_remaining_time() {
    let config = LlmRetryConfig {
        max_retry_elapsed: Duration::from_secs(3),
        ..Default::default()
    };
    let mut started = None;
    assert_eq!(remaining_retry_time(&config, started), None);
    assert_eq!(
        reserve_retry_wait(&config, &mut started, Duration::from_secs(3)),
        None
    );
    assert_eq!(
        reserve_retry_wait(&config, &mut started, Duration::from_secs(2)),
        Some(Duration::from_secs(2))
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        remaining_retry_time(&config, started),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        reserve_retry_wait(&config, &mut started, Duration::from_secs(1)),
        None
    );
    assert_eq!(
        reserve_retry_wait(&config, &mut started, Duration::from_millis(999)),
        Some(Duration::from_millis(999))
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        reserve_retry_wait(&config, &mut started, Duration::ZERO),
        None
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(remaining_retry_time(&config, started), None);
}

#[tokio::test(start_paused = true)]
async fn next_retry_wait_honors_retry_after_only_within_budget() {
    let config = LlmRetryConfig {
        initial_backoff: Duration::from_millis(100),
        jitter_factor: 0.0,
        max_retry_elapsed: Duration::from_secs(30),
        ..LlmRetryConfig::default()
    };
    let backoff = config.calculate_backoff(0);
    assert_eq!(next_retry_wait(&config, None, 0, None), backoff);
    assert_eq!(
        next_retry_wait(&config, None, 0, Some(7)),
        Duration::from_secs(7)
    );
    assert_eq!(next_retry_wait(&config, None, 0, Some(120)), backoff);

    let started = Some(crate::rt::Instant::now());
    tokio::time::advance(Duration::from_secs(25)).await;
    assert_eq!(next_retry_wait(&config, started, 0, Some(7)), backoff);
    assert_eq!(
        next_retry_wait(&config, started, 0, Some(4)),
        Duration::from_secs(4)
    );
}
