//! Host-owned checkpoints for provider recovery. A parked retry is control
//! flow, never a failed turn or a suspended language-runtime stack.
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::llm_retry::{LlmRetryConfig, RetryMetadata};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::Duration};

/// Secret-free, versioned continuation of one turn's reasoning iteration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryCheckpoint {
    pub version: u32,
    pub attempts: u32,
    pub started_at_ms: i64,
    pub expires_at_ms: i64,
    pub wake_at_ms: i64,
    pub total_wait_ms: u64,
    pub policy: LlmRetryConfig,
}

/// The session authority supplies time, fenced persistence and a durable wake.
#[async_trait]
pub trait RetryHost: Send + Sync {
    async fn now_ms(&self) -> Result<i64>;
    async fn load(&self, operation: &str) -> Result<Option<RetryCheckpoint>>;
    /// Persist before arming. A lost owner between these steps must still have
    /// its ordinary lease-expiry recovery obligation.
    async fn save(&self, operation: &str, checkpoint: &RetryCheckpoint) -> Result<()>;
    async fn schedule(&self, at_ms: i64) -> Result<()>;
}

tokio::task_local! {
    static HOST: Arc<dyn RetryHost>;
    static ATTEMPT: bool;
}

pub async fn scope<F: Future>(host: Arc<dyn RetryHost>, future: F) -> F::Output {
    HOST.scope(host, future).await
}

/// Only the shared Reason boundary opts into single-attempt driver behavior;
/// installing a host must not change arbitrary lifecycle or tool calls.
pub async fn attempt<F: Future>(future: F) -> F::Output {
    ATTEMPT
        .scope(HOST.try_with(|_| true).unwrap_or(false), future)
        .await
}

pub fn single_attempt() -> bool {
    ATTEMPT.try_with(|enabled| *enabled).unwrap_or(false)
}

#[derive(Debug, thiserror::Error)]
#[error("execution parked until a durable wakeup")]
struct Parked;

pub fn is_parked(error: &AgentLoopError) -> bool {
    matches!(error, AgentLoopError::Internal(inner) if inner.is::<Parked>())
}

fn parked() -> AgentLoopError {
    AgentLoopError::Internal(Parked.into())
}

#[derive(Debug, thiserror::Error)]
#[error("provider request can be retried before streaming")]
struct RetryableStart(AgentLoopError, Option<LlmRetryConfig>);

/// Whether this failure already carries authorization for a safe retry.
pub fn is_retryable_start(error: &AgentLoopError) -> bool {
    matches!(error, AgentLoopError::Internal(inner) if inner.is::<RetryableStart>())
}

/// Call only at a safe pre-stream boundary. Mid-stream failures and requests
/// with provider-executed side effects must never be converted to this marker.
pub fn retryable_start(error: AgentLoopError) -> AgentLoopError {
    if single_attempt() && error.is_transient_llm_error() {
        AgentLoopError::Internal(RetryableStart(error, None).into())
    } else {
        error
    }
}

/// Preserve an explicitly configured driver policy when Engine owns waits.
pub fn retryable_start_with_policy(
    error: AgentLoopError,
    policy: &LlmRetryConfig,
) -> AgentLoopError {
    if single_attempt() && error.is_transient_llm_error() && policy.max_retries > 0 {
        AgentLoopError::Internal(RetryableStart(error, Some(policy.clone())).into())
    } else {
        error
    }
}

pub fn take_retryable_start(
    error: AgentLoopError,
) -> std::result::Result<(AgentLoopError, Option<LlmRetryConfig>), AgentLoopError> {
    match error {
        AgentLoopError::Internal(inner) => match inner.downcast::<RetryableStart>() {
            Ok(start) => Ok((start.0, start.1)),
            Err(inner) => Err(AgentLoopError::Internal(inner)),
        },
        error => Err(error),
    }
}

/// Remove retry authorization at a request boundary with provider-side effects.
pub fn without_retry(error: AgentLoopError) -> AgentLoopError {
    match take_retryable_start(error) {
        Ok((error, _)) | Err(error) => error,
    }
}

fn metadata(checkpoint: &RetryCheckpoint) -> RetryMetadata {
    RetryMetadata {
        attempts: checkpoint.attempts,
        total_retry_wait: Duration::from_millis(checkpoint.total_wait_ms),
        ..Default::default()
    }
}

/// Restore the generation's count and wall time into the existing bounded
/// streaming loop. In-flight attempts remain bounded after process loss too.
pub async fn restore(
    operation: &str,
    fallback: &LlmRetryConfig,
) -> Result<(LlmRetryConfig, RetryMetadata, Option<crate::rt::Instant>)> {
    let Ok(host) = HOST.try_with(Arc::clone) else {
        return Ok((fallback.clone(), RetryMetadata::default(), None));
    };
    let Some(checkpoint) = host.load(operation).await? else {
        return Ok((fallback.clone(), RetryMetadata::default(), None));
    };
    let now = host.now_ms().await?;
    let mut policy = checkpoint.policy.clone();
    // A new isolate's monotonic clock may start at zero. Reconstruct a
    // remaining window instead of subtracting persisted time from that origin.
    policy.max_retry_elapsed =
        Duration::from_millis(checkpoint.expires_at_ms.saturating_sub(now).max(0) as u64);
    let mut restored = metadata(&checkpoint);
    restored.total_retry_elapsed =
        Duration::from_millis(now.saturating_sub(checkpoint.started_at_ms).max(0) as u64);
    Ok((policy, restored, Some(crate::rt::Instant::now())))
}

/// Before any provider I/O, re-arm early wakes or surface an exhausted budget
/// through the normal Reason failure path. Time comes from the authority.
pub async fn prepare(operation: &str) -> Result<Option<AgentLoopError>> {
    let Ok(host) = HOST.try_with(Arc::clone) else {
        return Ok(None);
    };
    let Some(checkpoint) = host.load(operation).await? else {
        return Ok(None);
    };
    if checkpoint.version != 1 {
        return Err(AgentLoopError::config(
            "unsupported provider retry checkpoint",
        ));
    }
    let now = host.now_ms().await?;
    if now >= checkpoint.expires_at_ms {
        return Ok(Some(
            AgentLoopError::llm_kind(
                LlmErrorKind::Unavailable,
                "provider recovery time budget exhausted",
            )
            .with_retry_metadata(&metadata(&checkpoint)),
        ));
    }
    if now < checkpoint.wake_at_ms {
        host.schedule(checkpoint.wake_at_ms).await?;
        return Err(parked());
    }
    Ok(None)
}

/// One budget spans driver sends, pre-first-event reconnects and Reason-level
/// retries. Persisting the selected deadline also preserves jitter on restart.
pub async fn retry(
    operation: &str,
    config: &LlmRetryConfig,
    error: AgentLoopError,
) -> AgentLoopError {
    match retain_retry(operation, config, error).await {
        Ok(error) | Err(error) => error,
    }
}

async fn retain_retry(
    operation: &str,
    config: &LlmRetryConfig,
    error: AgentLoopError,
) -> Result<AgentLoopError> {
    let Ok(host) = HOST.try_with(Arc::clone) else {
        return Ok(error);
    };
    let now = host.now_ms().await?;
    let mut state = host.load(operation).await?.unwrap_or(RetryCheckpoint {
        version: 1,
        attempts: 0,
        started_at_ms: now,
        expires_at_ms: now.saturating_add(
            i64::try_from(config.max_retry_elapsed.as_millis()).unwrap_or(i64::MAX),
        ),
        wake_at_ms: now,
        total_wait_ms: 0,
        policy: config.clone(),
    });
    if state.version != 1 {
        return Err(AgentLoopError::config(
            "unsupported provider retry checkpoint",
        ));
    }
    let remaining = state.expires_at_ms.saturating_sub(now).max(0) as u64;
    let config = &state.policy;
    if state.attempts >= config.max_retries || remaining == 0 {
        return Ok(error.with_retry_metadata(&metadata(&state)));
    }
    let backoff = config.calculate_backoff(state.attempts);
    let wait = error
        .retry_after_secs()
        .filter(|secs| *secs > 0 && *secs <= 60)
        .map(Duration::from_secs)
        .filter(|wait| wait.as_millis() < u128::from(remaining))
        .unwrap_or(backoff);
    let wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX).max(1);
    if wait_ms >= remaining {
        return Ok(error.with_retry_metadata(&metadata(&state)));
    }
    state.attempts += 1;
    state.total_wait_ms = state.total_wait_ms.saturating_add(wait_ms);
    state.wake_at_ms = now.saturating_add(i64::try_from(wait_ms).unwrap_or(i64::MAX));
    host.save(operation, &state).await?;
    host.schedule(state.wake_at_ms).await?;
    Ok(parked())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    };

    #[derive(Default)]
    struct Host {
        now: AtomicI64,
        saved: Mutex<Option<RetryCheckpoint>>,
        alarm: AtomicI64,
        fail_alarm: AtomicBool,
    }
    #[async_trait]
    impl RetryHost for Host {
        async fn now_ms(&self) -> Result<i64> {
            Ok(self.now.load(Ordering::SeqCst))
        }
        async fn load(&self, _: &str) -> Result<Option<RetryCheckpoint>> {
            Ok(self.saved.lock().unwrap().clone())
        }
        async fn save(&self, _: &str, checkpoint: &RetryCheckpoint) -> Result<()> {
            *self.saved.lock().unwrap() = Some(checkpoint.clone());
            Ok(())
        }
        async fn schedule(&self, at: i64) -> Result<()> {
            if self.fail_alarm.load(Ordering::SeqCst) {
                return Err(AgentLoopError::store("alarm unavailable"));
            }
            self.alarm.store(at, Ordering::SeqCst);
            Ok(())
        }
    }
    fn failure() -> AgentLoopError {
        AgentLoopError::llm_kind(LlmErrorKind::Unavailable, "temporary")
    }
    fn policy() -> LlmRetryConfig {
        LlmRetryConfig {
            jitter_factor: 0.0,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn reopening_preserves_deadline_and_exhausts_budget_without_a_timer() {
        let host = Arc::new(Host::default());
        let config = policy();
        for attempt in 1..=2 {
            scope(host.clone(), async {
                assert!(prepare("turn/1").await.unwrap().is_none());
                assert!(is_parked(&retry("turn/1", &config, failure()).await));
            })
            .await;
            let saved = host.saved.lock().unwrap().clone().unwrap();
            assert_eq!(saved.attempts, attempt);
            assert_eq!(host.alarm.load(Ordering::SeqCst), saved.wake_at_ms);
            scope(host.clone(), async {
                assert!(is_parked(&prepare("turn/1").await.unwrap_err()));
            })
            .await;
            host.now.store(saved.wake_at_ms, Ordering::SeqCst);
        }
        scope(host.clone(), async {
            let exhausted = retry("turn/1", &config, failure()).await;
            assert!(!is_parked(&exhausted));
            assert_eq!(exhausted.llm_retry_attempts(), 2);
        })
        .await;
    }

    #[tokio::test]
    async fn elapsed_wall_time_prevents_another_send_after_reopen() {
        let host = Arc::new(Host::default());
        scope(host.clone(), async {
            assert!(is_parked(&retry("turn/1", &policy(), failure()).await));
        })
        .await;
        host.now.store(30_001, Ordering::SeqCst);
        scope(host, async {
            let exhausted = prepare("turn/1").await.unwrap().unwrap();
            assert!(exhausted.llm_retry_handled());
            assert_eq!(exhausted.llm_retry_attempts(), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn restored_attempts_use_the_authority_remaining_budget_on_a_new_clock() {
        let host = Arc::new(Host::default());
        scope(host.clone(), async {
            assert!(is_parked(&retry("turn/1", &policy(), failure()).await));
        })
        .await;
        host.now.store(12_500, Ordering::SeqCst);
        scope(host, async {
            let (config, metadata, started) = restore("turn/1", &policy()).await.unwrap();
            assert_eq!(config.max_retry_elapsed, Duration::from_millis(17_500));
            assert_eq!(metadata.total_retry_elapsed, Duration::from_millis(12_500));
            assert_eq!(metadata.attempts, 1);
            assert!(
                crate::llm_retry::remaining_retry_time(&config, started).unwrap()
                    <= Duration::from_millis(17_500)
            );
        })
        .await;
    }

    #[tokio::test]
    async fn checkpoint_survives_alarm_failure_and_next_owner_repairs_it() {
        let host = Arc::new(Host::default());
        host.fail_alarm.store(true, Ordering::SeqCst);
        scope(host.clone(), async {
            assert!(matches!(
                retry("turn/1", &policy(), failure()).await,
                AgentLoopError::MessageStore(_)
            ));
        })
        .await;
        assert!(host.saved.lock().unwrap().is_some());
        host.fail_alarm.store(false, Ordering::SeqCst);
        scope(host.clone(), async {
            assert!(is_parked(&prepare("turn/1").await.unwrap_err()));
        })
        .await;
        assert_eq!(host.alarm.load(Ordering::SeqCst), 1000);
    }

    #[tokio::test]
    async fn host_scope_only_changes_provider_calls_inside_a_reason_attempt() {
        let host = Arc::new(Host::default());
        assert!(!single_attempt());
        scope(host, async {
            assert!(!single_attempt());
            attempt(async {
                assert!(single_attempt());
            })
            .await;
            assert!(!single_attempt());
        })
        .await;
        attempt(async {
            assert!(!single_attempt());
        })
        .await;
    }
}
