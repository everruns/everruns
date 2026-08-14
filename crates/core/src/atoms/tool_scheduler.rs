//! Tool execution scheduler for ActAtom.
//!
//! ActAtom receives a batch of tool calls that the model emitted in a single
//! turn. Historically every server-side call in that batch was run with a flat
//! `join_all`, i.e. unconditionally concurrent. That has two problems:
//!
//! 1. **Mutation conflicts.** Two calls that write the same shared session
//!    resource (two file edits, two SQL mutations) could interleave and race.
//! 2. **No concurrency bound.** A batch of N calls fans out to N in-flight
//!    executions with no cap, which can overwhelm providers, pools, or the box.
//!
//! This module decides *how* the batch runs, based on per-tool metadata
//! (`ToolHints`), while leaving *what each call does* to the caller:
//!
//! - Calls that share a non-empty
//!   [`ToolHints::concurrency_class`](everruns_provider::tool_types::ToolHints::concurrency_class)
//!   are
//!   serialized in arrival order; calls in different classes (or with no class)
//!   run concurrently. Read-only tools declare no class and always parallelize.
//! - A global concurrency cap bounds the number of simultaneously executing
//!   calls (configurable via `EVERRUNS_ACT_MAX_TOOL_CONCURRENCY`).
//! - The model/runtime can force fully sequential execution via
//!   [`ScheduleConfig::serialize_all`] (wired to the request's
//!   `parallel_tool_calls = false`).
//!
//! CPU-offload of individual heavy tools is intentionally *not* handled here:
//! it belongs to the per-call execution future (see ActAtom's
//! `execute_single_tool`, which spawns `cpu_bound` tools onto their own task).
//! Keeping that concern out of the scheduler lets the scheduler operate on
//! borrowed, non-`'static` futures.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use futures::future::join_all;
use tokio::sync::Semaphore;

/// Default cap on simultaneously executing tool calls within a single act.
///
/// Chosen to comfortably cover normal fan-out (handfuls of parallel reads or
/// subagent spawns) while protecting shared resources from pathological
/// batches. Override per-process with `EVERRUNS_ACT_MAX_TOOL_CONCURRENCY`.
pub const DEFAULT_MAX_TOOL_CONCURRENCY: usize = 32;

/// Resolve the effective tool concurrency cap, honoring the
/// `EVERRUNS_ACT_MAX_TOOL_CONCURRENCY` env override (clamped to >= 1).
pub fn configured_max_tool_concurrency() -> usize {
    std::env::var("EVERRUNS_ACT_MAX_TOOL_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.max(1))
        .unwrap_or(DEFAULT_MAX_TOOL_CONCURRENCY)
}

/// Configuration for a single scheduling pass.
#[derive(Debug, Clone, Copy)]
pub struct ScheduleConfig {
    /// Maximum number of tool calls executing simultaneously.
    pub max_concurrency: usize,
    /// When true, run every call strictly sequentially in arrival order,
    /// ignoring concurrency classes entirely. Set when the request asked to
    /// disable parallel tool use.
    pub serialize_all: bool,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            max_concurrency: configured_max_tool_concurrency(),
            serialize_all: false,
        }
    }
}

/// Execute `n` tool calls according to `config` and per-call conflict classes.
///
/// - `classes[i]` is the conflict key for call `i`: `None`, empty, or a missing
///   entry (a `classes` slice shorter than `n`) means the call has no conflicts
///   and may run concurrently with anything; calls sharing the same
///   `Some(class)` run sequentially in arrival order. `classes` should normally
///   have exactly `n` entries; extra entries are ignored.
/// - `run(i)` produces the future that executes call `i`. It is invoked lazily,
///   only once the scheduler is ready to start that call (after acquiring a
///   concurrency permit and, for a serialized class, after the previous call in
///   the class has finished).
///
/// Results are returned in the original `0..n` order regardless of completion
/// order. `run` is never invoked more than once per index.
pub async fn schedule<R, MkFut, Fut>(
    n: usize,
    classes: &[Option<String>],
    config: ScheduleConfig,
    run: MkFut,
) -> Vec<R>
where
    MkFut: Fn(usize) -> Fut,
    Fut: Future<Output = R>,
{
    if n == 0 {
        return Vec::new();
    }

    // Fully sequential mode: simplest correct behavior, used when the request
    // disabled parallel tool use.
    if config.serialize_all || n == 1 {
        let mut results = Vec::with_capacity(n);
        for i in 0..n {
            results.push(run(i).await);
        }
        return results;
    }

    // Build serialized groups: each named class collects its call indices in
    // arrival order; class-less calls each form their own singleton group so
    // they run with full parallelism. `groups` preserves first-seen order for
    // deterministic, arrival-ordered scheduling.
    //
    // Iterate `0..n` (not `classes.iter()`) and treat any missing/empty entry
    // as "no class": every call index is scheduled exactly once regardless of
    // the `classes` slice length, so a short slice can never silently drop a
    // call in release builds (where the `debug_assert_eq!` above is gone).
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut class_index: HashMap<&str, usize> = HashMap::new();
    for i in 0..n {
        match classes
            .get(i)
            .and_then(|class| class.as_deref())
            .filter(|class| !class.is_empty())
        {
            Some(key) => {
                if let Some(&g) = class_index.get(key) {
                    groups[g].push(i);
                } else {
                    class_index.insert(key, groups.len());
                    groups.push(vec![i]);
                }
            }
            None => groups.push(vec![i]),
        }
    }

    let semaphore = Arc::new(Semaphore::new(config.max_concurrency.max(1)));
    let run = &run;

    // Each group runs its members sequentially; groups run concurrently. A
    // permit is held only for the duration of an individual call, so the cap
    // bounds total in-flight executions across all groups.
    let group_futures = groups.into_iter().map(|group| {
        let semaphore = semaphore.clone();
        async move {
            let mut out: Vec<(usize, R)> = Vec::with_capacity(group.len());
            for idx in group {
                let permit = semaphore
                    .acquire()
                    .await
                    .expect("tool scheduler semaphore is never closed");
                let result = run(idx).await;
                drop(permit);
                out.push((idx, result));
            }
            out
        }
    });

    // Flatten and restore original ordering.
    let mut indexed: Vec<(usize, R)> = join_all(group_futures)
        .await
        .into_iter()
        .flatten()
        .collect();
    indexed.sort_by_key(|(idx, _)| *idx);
    indexed.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Records start/finish ordering so tests can assert on concurrency.
    #[derive(Default)]
    struct Tracker {
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        events: Mutex<Vec<String>>,
    }

    impl Tracker {
        fn enter(&self, label: &str) {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            self.events.lock().unwrap().push(format!("start:{label}"));
        }
        fn exit(&self, label: &str) {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.events.lock().unwrap().push(format!("end:{label}"));
        }
    }

    #[tokio::test]
    async fn preserves_input_order() {
        let classes = vec![None, None, None];
        let results = schedule(
            3,
            &classes,
            ScheduleConfig::default(),
            |i| async move { i * 10 },
        )
        .await;
        assert_eq!(results, vec![0, 10, 20]);
    }

    #[tokio::test]
    async fn distinct_classes_run_concurrently() {
        let tracker = Arc::new(Tracker::default());
        // Three class-less calls => three singleton groups => full parallelism.
        let classes = vec![None, None, None];
        let t = tracker.clone();
        schedule(3, &classes, ScheduleConfig::default(), |i| {
            let t = t.clone();
            async move {
                t.enter(&i.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                t.exit(&i.to_string());
            }
        })
        .await;
        assert_eq!(tracker.max_in_flight.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn same_class_serializes_in_arrival_order() {
        let tracker = Arc::new(Tracker::default());
        // All three share a class => must run one at a time, in order 0,1,2.
        let classes = vec![
            Some("fs".to_string()),
            Some("fs".to_string()),
            Some("fs".to_string()),
        ];
        let t = tracker.clone();
        schedule(3, &classes, ScheduleConfig::default(), |i| {
            let t = t.clone();
            async move {
                t.enter(&i.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                t.exit(&i.to_string());
            }
        })
        .await;
        assert_eq!(tracker.max_in_flight.load(Ordering::SeqCst), 1);
        let events = tracker.events.lock().unwrap().clone();
        assert_eq!(
            events,
            vec!["start:0", "end:0", "start:1", "end:1", "start:2", "end:2"]
        );
    }

    #[tokio::test]
    async fn mixed_classes_serialize_within_parallelize_across() {
        let tracker = Arc::new(Tracker::default());
        // Two "fs" calls (serialized) + two class-less calls (parallel).
        // Peak concurrency should be 3: one fs + two free.
        let classes = vec![Some("fs".to_string()), None, Some("fs".to_string()), None];
        let t = tracker.clone();
        schedule(4, &classes, ScheduleConfig::default(), |i| {
            let t = t.clone();
            async move {
                t.enter(&i.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                t.exit(&i.to_string());
            }
        })
        .await;
        assert_eq!(tracker.max_in_flight.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn global_cap_bounds_concurrency() {
        let tracker = Arc::new(Tracker::default());
        let classes = vec![None; 10];
        let cfg = ScheduleConfig {
            max_concurrency: 2,
            serialize_all: false,
        };
        let t = tracker.clone();
        schedule(10, &classes, cfg, |i| {
            let t = t.clone();
            async move {
                t.enter(&i.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                t.exit(&i.to_string());
            }
        })
        .await;
        assert!(tracker.max_in_flight.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn shorter_classes_slice_schedules_all_calls() {
        // A `classes` slice shorter than `n` must not drop calls: missing
        // entries are treated as "no class" and every index still runs.
        let classes = vec![Some("ws".to_string())]; // len 1, but n = 3
        let results = schedule(3, &classes, ScheduleConfig::default(), |i| async move { i }).await;
        assert_eq!(results, vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn serialize_all_ignores_classes() {
        let tracker = Arc::new(Tracker::default());
        let classes = vec![None, None, None];
        let cfg = ScheduleConfig {
            max_concurrency: 8,
            serialize_all: true,
        };
        let t = tracker.clone();
        let results = schedule(3, &classes, cfg, |i| {
            let t = t.clone();
            async move {
                t.enter(&i.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                t.exit(&i.to_string());
                i
            }
        })
        .await;
        assert_eq!(results, vec![0, 1, 2]);
        assert_eq!(tracker.max_in_flight.load(Ordering::SeqCst), 1);
    }
}
