//! Native clock for the portable executor-poll CPU observer.

use crate::engine::ThreadCpuClock;
use std::sync::Arc;

pub(super) fn clock() -> Option<Arc<dyn ThreadCpuClock>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        Some(Arc::new(NativeThreadCpuClock))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct NativeThreadCpuClock;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl ThreadCpuClock for NativeThreadCpuClock {
    fn thread_cpu_time_ns(&self) -> Option<u64> {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `time` is a valid, writable timespec for the duration of the
        // call. The OS clock needs no privilege; failure leaves it unknown.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
            return None;
        }
        let seconds = u64::try_from(time.tv_sec).ok()?;
        let nanos = u64::try_from(time.tv_nsec).ok()?;
        if nanos >= 1_000_000_000 {
            return None;
        }
        seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    #[test]
    fn native_clock_observes_finite_current_thread_cpu() {
        let clock = clock().unwrap();
        let start = clock.thread_cpu_time_ns().expect("native thread clock");
        // Finite local work, no stress probe or external request.
        let mut value = 1_u64;
        for i in 0..10_000 {
            value = std::hint::black_box(value.wrapping_add(i));
        }
        std::hint::black_box(value);
        let end = clock.thread_cpu_time_ns().expect("native thread clock");
        assert!(end > start);
    }
}
