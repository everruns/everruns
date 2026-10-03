//! Portable async runtime primitives: time, timers and task spawning.
//!
//! The execution algorithms in `everruns-provider`, `everruns-core` and
//! `everruns-engine` run on Tokio in every native host. They also run inside a
//! JavaScript isolate (a celld or Cloudflare Durable Object, a browser) built
//! for `wasm32-unknown-unknown`, where there is no Tokio runtime, no thread to
//! spawn on, and `std::time::Instant::now` panics.
//!
//! Decisions:
//! - Native is Tokio, unchanged: the re-exports are Tokio's own types, so
//!   paused-clock tests (`tokio::time::advance`) keep working.
//! - On `wasm32-unknown-unknown`, time comes from the host clock
//!   (`performance.now()` through `web-time`), timers from `setTimeout`
//!   (`gloo-timers`) and tasks from the isolate's microtask queue
//!   (`wasm-bindgen-futures`).
//! - JavaScript-backed futures are `!Send`, while every engine contract is
//!   `Send` so native hosts can use the multi-thread runtime. On the wasm
//!   target there is exactly one thread, so [`AssertSend`] marks them `Send`
//!   there and only there.

pub use std::time::Duration;

/// `wasm32-unknown-unknown`: the isolate target.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod imp {
    use std::fmt;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use futures::channel::oneshot;
    use futures::future::{AbortHandle, Abortable};

    pub use web_time::Instant;

    /// A future that is `Send` because the target has one thread.
    ///
    /// Sound only on `wasm32-unknown-unknown`, where no value can cross a
    /// thread; the type does not exist on any other target.
    pub struct AssertSend<F>(pub F);

    // SAFETY: compiled only for wasm32-unknown-unknown, which has a single
    // thread (no `std::thread`, no shared-memory workers in this runtime), so
    // a value can never be sent to or shared with another thread.
    unsafe impl<F> Send for AssertSend<F> {}
    // SAFETY: as above.
    unsafe impl<F> Sync for AssertSend<F> {}

    impl<F: Future> Future for AssertSend<F> {
        type Output = F::Output;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
            // SAFETY: structural pinning of the only field.
            unsafe { self.map_unchecked_mut(|this| &mut this.0) }.poll(cx)
        }
    }

    /// A timer that completes after a duration.
    pub struct Sleep(AssertSend<gloo_timers::future::TimeoutFuture>);

    impl Future for Sleep {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            Pin::new(&mut self.0).poll(cx)
        }
    }

    /// Wait for `duration`.
    pub fn sleep(duration: Duration) -> Sleep {
        let millis = u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
        Sleep(AssertSend(gloo_timers::future::TimeoutFuture::new(millis)))
    }

    /// The deadline passed before the future completed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Elapsed(());

    impl fmt::Display for Elapsed {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("deadline has elapsed")
        }
    }

    impl std::error::Error for Elapsed {}

    /// Run `future`, giving up after `duration`.
    pub async fn timeout<F: Future>(duration: Duration, future: F) -> Result<F::Output, Elapsed> {
        use futures::future::{Either, select};
        let future = std::pin::pin!(future);
        match select(future, sleep(duration)).await {
            Either::Left((output, _)) => Ok(output),
            Either::Right(((), _)) => Err(Elapsed(())),
        }
    }

    /// The task was aborted before it finished.
    #[derive(Debug)]
    pub struct JoinError(());

    impl fmt::Display for JoinError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("task was cancelled")
        }
    }

    impl std::error::Error for JoinError {}

    /// A spawned task. Dropping it detaches the task, as with Tokio.
    pub struct JoinHandle<T> {
        output: oneshot::Receiver<T>,
        abort: AbortHandle,
        done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl<T> JoinHandle<T> {
        pub fn abort(&self) {
            self.abort.abort();
        }

        pub fn is_finished(&self) -> bool {
            self.done.load(std::sync::atomic::Ordering::Relaxed) || self.abort.is_aborted()
        }
    }

    impl<T> Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.output)
                .poll(cx)
                .map(|result| result.map_err(|_| JoinError(())))
        }
    }

    /// Run `future` on the isolate's task queue.
    pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        let (abort, registration) = AbortHandle::new_pair();
        let (tx, rx) = oneshot::channel();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = done.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(output) = Abortable::new(future, registration).await {
                let _ = tx.send(output);
            }
            finished.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        JoinHandle {
            output: rx,
            abort,
            done,
        }
    }
}

/// Every other target: Tokio.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod imp {
    pub use tokio::task::{JoinError, JoinHandle, spawn};
    pub use tokio::time::error::Elapsed;
    pub use tokio::time::{Instant, Sleep, sleep, timeout};
}

pub use imp::*;

/// A periodic tick that skips missed ticks, like Tokio's `interval` with
/// `MissedTickBehavior::Skip`: the first [`Interval::tick`] completes at
/// once, later ones one `period` apart. On native targets it is Tokio's own
/// interval, so its scheduling (and paused-clock behavior) is unchanged.
pub struct Interval {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    inner: tokio::time::Interval,
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    period: Duration,
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    next: Option<std::pin::Pin<Box<Sleep>>>,
}

impl Interval {
    pub fn new(period: Duration) -> Self {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let mut inner = tokio::time::interval(period);
            inner.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            Self { inner }
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            Self { period, next: None }
        }
    }

    /// Wait for the next tick. Cancel-safe: a dropped `tick` keeps the
    /// schedule.
    pub async fn tick(&mut self) {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            self.inner.tick().await;
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            if let Some(next) = self.next.as_mut() {
                next.as_mut().await;
            }
            self.next = Some(Box::pin(sleep(self.period)));
        }
    }
}

/// Seconds since the Unix epoch on the host clock.
pub fn unix_now_secs() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
