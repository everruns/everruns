//! CPU consumed while polling one executor future, excluding time suspended.

use std::future::{Future, poll_fn};

/// Host-provided current-thread CPU clock. Values need only be comparable
/// within a single future poll; the task may migrate between worker threads.
/// Missing/failed observations must return `None`, never a fabricated zero.
pub trait ThreadCpuClock: Send + Sync {
    /// Return the calling thread's cumulative CPU nanoseconds, or unknown.
    fn thread_cpu_time_ns(&self) -> Option<u64>;
}

pub(crate) async fn observe_cpu<F: Future>(
    future: F,
    clock: Option<&dyn ThreadCpuClock>,
) -> (F::Output, Option<u64>) {
    let Some(clock) = clock else {
        return (future.await, None);
    };
    let mut future = std::pin::pin!(future);
    let mut cpu_ns = Some(0_u64);
    let output = poll_fn(|cx| {
        let start = clock.thread_cpu_time_ns();
        let result = future.as_mut().poll(cx);
        let end = clock.thread_cpu_time_ns();
        // Failure is sticky: reporting a partial sum as complete would
        // understate the observed scope. Never compare clocks across polls.
        cpu_ns = cpu_ns
            .zip(start.zip(end))
            .and_then(|(sum, (start, end))| sum.checked_add(end.checked_sub(start)?));
        result
    })
    .await;
    (output, cpu_ns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex, task::Poll};

    struct Clock(Mutex<VecDeque<Option<u64>>>);
    impl ThreadCpuClock for Clock {
        fn thread_cpu_time_ns(&self) -> Option<u64> {
            self.0.lock().unwrap().pop_front().expect("clock sample")
        }
    }
    async fn two_polls(samples: [Option<u64>; 4]) -> Option<u64> {
        let clock = Clock(Mutex::new(samples.into()));
        let mut first = true;
        let future = poll_fn(|cx| {
            if std::mem::take(&mut first) {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        });
        observe_cpu(future, Some(&clock)).await.1
    }

    #[tokio::test]
    async fn cpu_is_summed_within_polls_not_across_waits_or_threads() {
        // The second poll can run on a thread with an unrelated CPU epoch.
        assert_eq!(
            two_polls([Some(100), Some(110), Some(5), Some(25)]).await,
            Some(30)
        );
        assert_eq!(
            two_polls([Some(100), Some(100), Some(9000), Some(9000)]).await,
            Some(0)
        );
    }
    #[tokio::test]
    async fn missing_reversed_or_overflowed_clock_is_unknown() {
        for samples in [
            [None, Some(10), Some(20), Some(30)],
            [Some(20), Some(10), Some(20), Some(30)],
            [Some(0), Some(u64::MAX), Some(0), Some(1)],
        ] {
            assert_eq!(two_polls(samples).await, None);
        }
        assert_eq!(observe_cpu(async { 7 }, None).await, (7, None));
    }
}
