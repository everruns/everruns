use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt;
use tokio::task::JoinHandle;

/// Restart behavior for a supervised task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartPolicy {
    /// Log task termination and keep the handle for shutdown aborts, but do not
    /// restart it.
    Never,
    /// Restart after any normal return or panic. Long-lived server control-plane
    /// loops use this so one panic does not permanently disable a subsystem.
    Always { delay: Duration },
}

impl RestartPolicy {
    pub const fn always_after(delay: Duration) -> Self {
        Self::Always { delay }
    }
}

#[derive(Debug)]
struct SupervisedTaskHandle {
    name: &'static str,
    handle: JoinHandle<()>,
}

/// Retains handles for server-owned background tasks.
///
/// Dropping the supervisor aborts any still-running task, which gives startup
/// phases a single owner for background work instead of scattering detached
/// `tokio::spawn` calls throughout `app_builder`.
#[derive(Default, Debug)]
pub struct TaskSupervisor {
    handles: Vec<SupervisedTaskHandle>,
}

impl TaskSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn track(&mut self, name: &'static str, handle: JoinHandle<()>) {
        self.handles.push(SupervisedTaskHandle { name, handle });
    }

    pub fn track_optional(&mut self, name: &'static str, handle: Option<JoinHandle<()>>) {
        if let Some(handle) = handle {
            self.track(name, handle);
        }
    }

    pub fn spawn<F, Fut>(&mut self, name: &'static str, restart: RestartPolicy, mut task: F)
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::spawn(async move {
            loop {
                let result = AssertUnwindSafe(task()).catch_unwind().await;
                metrics::counter!("server_background_task_exits_total", "task" => name)
                    .increment(1);

                match result {
                    Ok(()) => tracing::warn!(task = name, "Supervised background task exited"),
                    Err(_) => {
                        metrics::counter!("server_background_task_panics_total", "task" => name)
                            .increment(1);
                        tracing::error!(task = name, "Supervised background task panicked");
                    }
                }

                match restart {
                    RestartPolicy::Never => break,
                    RestartPolicy::Always { delay } => {
                        metrics::counter!("server_background_task_restarts_total", "task" => name)
                            .increment(1);
                        tracing::warn!(
                            task = name,
                            restart_delay_ms = delay.as_millis() as u64,
                            "Restarting supervised background task"
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        });
        self.track(name, handle);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.handles.len()
    }
}

impl Drop for TaskSupervisor {
    fn drop(&mut self) {
        for task in &self.handles {
            if !task.handle.is_finished() {
                tracing::info!(task = task.name, "Aborting supervised background task");
                task.handle.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn tracks_spawned_handles() {
        let mut supervisor = TaskSupervisor::new();
        supervisor.track("test", tokio::spawn(async {}));
        assert_eq!(supervisor.len(), 1);
    }

    #[tokio::test]
    async fn restarts_panicking_tasks_when_configured() {
        let mut supervisor = TaskSupervisor::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let runs_for_task = runs.clone();

        supervisor.spawn(
            "restart-test",
            RestartPolicy::always_after(Duration::from_millis(1)),
            move || {
                let runs = runs_for_task.clone();
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    panic!("intentional test panic");
                }
            },
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            while runs.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("supervisor should restart a panicking task");
    }

    #[tokio::test]
    async fn drop_aborts_tracked_task() {
        let mut supervisor = TaskSupervisor::new();
        let handle = tokio::spawn(async {
            futures::future::pending::<()>().await;
        });
        let abort_handle = handle.abort_handle();

        supervisor.track("pending", handle);
        drop(supervisor);

        tokio::time::timeout(Duration::from_secs(1), async {
            while !abort_handle.is_finished() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("tracked task should be aborted when supervisor drops");
    }
}
