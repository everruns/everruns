// Mid-turn task wake delivery (EVE-681, part A).
//
// Wake-ups are a registry-level delivery policy: when a task emits qualifying
// outbound activity (see `TaskWakePolicy`), the owning session's agent is woken
// so it can react. Historically the only delivery path was a between-turn
// steering message — a parent that spawned background work finished its turn,
// idled, and only reacted on its next turn (see the Wake-ups section of
// `knowledge/runtime-resources/session-tasks.md`).
//
// `SessionWakeQueue` adds the *mid-turn* path. It is a per-session queue that
// sits behind the registry seam: the registry fans qualifying task transitions
// into it (via the EVE-729 `TaskTransitionObserver` seam), and the agentic turn
// loop drains it at each iteration boundary — before the next LLM call —
// injecting the wake payloads as context alongside the reloaded conversation.
//
// Exactly-once claim point
// -------------------------
// The queue is the single source of truth for an undelivered wake. Each real
// transition enqueues exactly one `PendingWake` (the registry guarantees one
// observer notification per real transition). `drain` atomically removes and
// returns a session's queued wakes under a single lock — that removal *is* the
// claim. A wake is therefore delivered mid-turn (drained by a running turn's
// next iteration) XOR queued for the next turn (drained by that turn's first
// iteration), never both:
//
//   * turn cancellation / seal / max-iterations: an undrained wake stays in the
//     queue and is delivered by the next turn's first drain (between-turn
//     fallback), because nothing removed it.
//   * a wake landing mid-loop is visible to the very next iteration, because the
//     loop drains at the top of every reason step.
//
// The queue is process-local. Within a single runtime/worker process it gives
// exactly-once delivery; durable exactly-once across a worker restart is a
// property of the *persistent* transition source (the durable signal store on
// the server path), not of this in-memory queue. The server durable-worker
// wiring is intentionally out of scope for part A — see the PR / spec notes.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::session_task::{SessionTask, TaskWakePolicy};
use crate::task_observer::{TaskTransition, TaskTransitionObserver};
use crate::typed_id::SessionId;

/// A wake destined for a session's running (or next) turn.
///
/// `text` is the rendered, model-facing payload — the task snapshot summary plus
/// (for `OnActivity`) its latest progress/detail. The loop injects it as a user
/// message so the next LLM call reacts to it alongside pending tool results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWake {
    pub task_id: String,
    pub session_id: SessionId,
    pub transition: TaskTransition,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

/// Render the wake text for a task transition under the task's `wake_policy`.
///
/// Returns `None` when the policy does not wake on this transition (so the
/// caller enqueues nothing). This encodes the same gating the between-turn
/// waker uses (`DbSessionTaskRegistry::maybe_wake_*`) so mid-turn and
/// between-turn delivery agree on *when* a wake fires:
///
///   * `Silent`     — never.
///   * `OnTerminal` — only on a terminal transition.
///   * `OnActivity` — terminal, `awaiting_input`, and outbound messages.
///
/// The text is rendered purely from the task snapshot: the terminal summary /
/// result path, the awaiting-input prompt, or the latest progress / detail for
/// a message. The full message thread stays in the task record; the wake tells
/// the agent to look.
pub fn wake_text_for(task: &SessionTask, transition: TaskTransition) -> Option<String> {
    match (task.wake_policy, transition) {
        (TaskWakePolicy::Silent, _) => None,
        (TaskWakePolicy::OnTerminal, TaskTransition::Terminal)
        | (TaskWakePolicy::OnActivity, TaskTransition::Terminal) => {
            let mut parts = vec![format!(
                "Task \"{}\" ({}) finished: {}.",
                task.display_name, task.id, task.state
            )];
            if let Some(summary) = &task.summary {
                parts.push(format!("- summary: {summary}"));
            }
            if let Some(result_path) = &task.result_path {
                parts.push(format!("- result_path: {result_path}"));
            }
            Some(parts.join("\n"))
        }
        (TaskWakePolicy::OnTerminal, _) => None,
        (TaskWakePolicy::OnActivity, TaskTransition::AwaitingInput) => {
            let prompt = task
                .input_request
                .as_ref()
                .map(|r| r.prompt.as_str())
                .unwrap_or("Task is awaiting input.");
            Some(format!(
                "Task \"{}\" ({}) is awaiting input: {}",
                task.display_name, task.id, prompt
            ))
        }
        (TaskWakePolicy::OnActivity, TaskTransition::Message) => {
            // Observers do not receive the message body (it is not on the task
            // snapshot); surface the latest progress/detail so the agent knows
            // what changed, and read the thread via `get_task` for the payload.
            let detail = task
                .state_detail
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| task.progress.as_ref().and_then(|p| p.label.as_deref()))
                .unwrap_or("structured progress update");
            Some(format!(
                "Task \"{}\" ({}) sent a message: {}",
                task.display_name, task.id, detail
            ))
        }
    }
}

/// Per-session mid-turn wake queue behind the registry seam.
///
/// Feed it by registering it as a [`TaskTransitionObserver`] on an
/// [`crate::task_observer::ObservingTaskRegistry`] (or the server's
/// `DbSessionTaskRegistry`). Drain it from the turn loop with [`Self::drain`].
#[derive(Default)]
pub struct SessionWakeQueue {
    queues: Mutex<HashMap<SessionId, VecDeque<PendingWake>>>,
}

impl SessionWakeQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a wake for `task`'s owning session if its policy wakes on
    /// `transition`. Returns `true` when a wake was enqueued.
    pub fn note_transition(&self, task: &SessionTask, transition: TaskTransition) -> bool {
        let Some(text) = wake_text_for(task, transition) else {
            return false;
        };
        let wake = PendingWake {
            task_id: task.id.clone(),
            session_id: task.session_id,
            transition,
            text,
            created_at: Utc::now(),
        };
        self.queues
            .lock()
            .expect("wake queue mutex poisoned")
            .entry(task.session_id)
            .or_default()
            .push_back(wake);
        true
    }

    /// Atomically remove and return all wakes queued for `session_id`.
    ///
    /// This is the exactly-once claim point: a drained wake is gone from the
    /// queue and can never be delivered again.
    pub fn drain(&self, session_id: SessionId) -> Vec<PendingWake> {
        let mut guard = self.queues.lock().expect("wake queue mutex poisoned");
        match guard.remove(&session_id) {
            Some(queue) => queue.into_iter().collect(),
            None => Vec::new(),
        }
    }

    /// Number of wakes currently queued for `session_id` (not consuming).
    pub fn pending_len(&self, session_id: SessionId) -> usize {
        self.queues
            .lock()
            .expect("wake queue mutex poisoned")
            .get(&session_id)
            .map_or(0, VecDeque::len)
    }

    /// Whether any wake is queued for `session_id` (not consuming).
    pub fn has_pending(&self, session_id: SessionId) -> bool {
        self.pending_len(session_id) > 0
    }
}

/// The queue is a [`TaskTransitionObserver`] so it can be attached to the same
/// seam the server webhook dispatcher uses (EVE-729). Terminal and
/// awaiting-input transitions render at full fidelity from the snapshot; a
/// message transition renders from the snapshot's progress/detail.
#[async_trait]
impl TaskTransitionObserver for SessionWakeQueue {
    async fn on_transition(
        &self,
        task: &SessionTask,
        transition: TaskTransition,
    ) -> anyhow::Result<()> {
        self.note_transition(task, transition);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_task::{
        CreateSessionTask, SessionTaskState, TaskInputRequest, new_session_task,
    };

    fn task_with_policy(policy: TaskWakePolicy) -> SessionTask {
        let session_id = SessionId::from_seed(1);
        let mut task = new_session_task(
            CreateSessionTask {
                id: Some("task_a".into()),
                session_id,
                kind: "subagent".into(),
                display_name: "Test Runner".into(),
                spec: serde_json::Value::Null,
                state: SessionTaskState::Running,
                links: Default::default(),
                wake_policy: policy,
            },
            Utc::now(),
        );
        task.state = SessionTaskState::Succeeded;
        task
    }

    #[test]
    fn policy_matrix_controls_rendering_and_enqueueing() {
        for (policy, allowed) in [
            (TaskWakePolicy::Silent, [false, false, false]),
            (TaskWakePolicy::OnTerminal, [true, false, false]),
            (TaskWakePolicy::OnActivity, [true, true, true]),
        ] {
            for (index, transition) in [
                TaskTransition::Terminal,
                TaskTransition::AwaitingInput,
                TaskTransition::Message,
            ]
            .into_iter()
            .enumerate()
            {
                let queue = SessionWakeQueue::new();
                let task = task_with_policy(policy);
                assert_eq!(wake_text_for(&task, transition).is_some(), allowed[index]);
                assert_eq!(queue.note_transition(&task, transition), allowed[index]);
                assert_eq!(queue.has_pending(task.session_id), allowed[index]);
                assert_eq!(
                    queue.pending_len(task.session_id),
                    usize::from(allowed[index])
                );
            }
        }
    }

    #[test]
    fn terminal_text_preserves_identity_status_and_optional_fields() {
        let mut task = task_with_policy(TaskWakePolicy::OnTerminal);
        assert_eq!(
            wake_text_for(&task, TaskTransition::Terminal).as_deref(),
            Some("Task \"Test Runner\" (task_a) finished: succeeded.")
        );
        task.summary = Some("all tests passed".into());
        task.result_path = Some("/.tasks/task_a/result.json".into());
        assert_eq!(
            wake_text_for(&task, TaskTransition::Terminal).as_deref(),
            Some(
                "Task \"Test Runner\" (task_a) finished: succeeded.\n- summary: all tests passed\n- result_path: /.tasks/task_a/result.json"
            )
        );
        task.state = SessionTaskState::Failed;
        task.summary = None;
        assert_eq!(
            wake_text_for(&task, TaskTransition::Terminal).as_deref(),
            Some(
                "Task \"Test Runner\" (task_a) finished: failed.\n- result_path: /.tasks/task_a/result.json"
            )
        );
    }

    #[test]
    fn activity_text_uses_input_prompt_and_detail_label_fallbacks() {
        let mut task = task_with_policy(TaskWakePolicy::OnActivity);
        task.input_request = Some(TaskInputRequest {
            id: "input_1".into(),
            prompt: "pick a branch".into(),
            expected: None,
        });
        assert_eq!(
            wake_text_for(&task, TaskTransition::AwaitingInput).as_deref(),
            Some("Task \"Test Runner\" (task_a) is awaiting input: pick a branch")
        );
        task.input_request = None;
        assert_eq!(
            wake_text_for(&task, TaskTransition::AwaitingInput).as_deref(),
            Some("Task \"Test Runner\" (task_a) is awaiting input: Task is awaiting input.")
        );
        for (detail, label, expected) in [
            (Some("iteration 4/10"), Some("label"), "iteration 4/10"),
            (Some(" \u{2003}"), Some("label"), "label"),
            (None, Some("label"), "label"),
            (None, None, "structured progress update"),
        ] {
            task.state_detail = detail.map(str::to_string);
            task.progress = label.map(|label| crate::session_task::TaskProgress {
                current: Some(4),
                total: Some(10),
                unit: Some("steps".into()),
                label: Some(label.into()),
            });
            assert_eq!(
                wake_text_for(&task, TaskTransition::Message).unwrap(),
                format!("Task \"Test Runner\" (task_a) sent a message: {expected}")
            );
        }
    }

    #[tokio::test]
    async fn observer_delivery_is_fifo_isolated_and_claimed_once() {
        let queue = SessionWakeQueue::new();
        let mut a = task_with_policy(TaskWakePolicy::OnActivity);
        let mut b = a.clone();
        b.id = "task_b".into();
        b.session_id = SessionId::from_seed(2);
        assert!(queue.drain(a.session_id).is_empty());
        queue
            .on_transition(&a, TaskTransition::Terminal)
            .await
            .unwrap();
        a.id = "task_next".into();
        queue
            .on_transition(&a, TaskTransition::Message)
            .await
            .unwrap();
        queue
            .on_transition(&b, TaskTransition::Terminal)
            .await
            .unwrap();
        assert_eq!(queue.pending_len(a.session_id), 2);
        assert_eq!(
            queue.pending_len(a.session_id),
            2,
            "inspection does not claim wakes"
        );
        let wakes = queue.drain(a.session_id);
        assert_eq!(
            wakes
                .iter()
                .map(|wake| (
                    wake.task_id.as_str(),
                    wake.session_id,
                    wake.transition,
                    wake.text.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                (
                    "task_a",
                    SessionId::from_seed(1),
                    TaskTransition::Terminal,
                    "Task \"Test Runner\" (task_a) finished: succeeded."
                ),
                (
                    "task_next",
                    SessionId::from_seed(1),
                    TaskTransition::Message,
                    "Task \"Test Runner\" (task_next) sent a message: structured progress update"
                ),
            ]
        );
        assert!(queue.drain(a.session_id).is_empty());
        assert!(!queue.has_pending(a.session_id));
        assert_eq!(queue.pending_len(b.session_id), 1);
        let other = queue.drain(b.session_id);
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].task_id, "task_b");
        assert_eq!(other[0].session_id, SessionId::from_seed(2));
    }

    #[test]
    fn concurrent_drains_claim_each_wake_once() {
        let queue = SessionWakeQueue::new();
        let mut task = task_with_policy(TaskWakePolicy::OnTerminal);
        for n in 0..32 {
            task.id = format!("task_{n}");
            queue.note_transition(&task, TaskTransition::Terminal);
        }
        let barrier = std::sync::Barrier::new(2);
        let mut ids = std::thread::scope(|scope| {
            let drain = || {
                barrier.wait();
                queue.drain(task.session_id)
            };
            let first = scope.spawn(drain);
            let second = scope.spawn(drain);
            first
                .join()
                .unwrap()
                .into_iter()
                .chain(second.join().unwrap())
                .map(|wake| wake.task_id)
                .collect::<Vec<_>>()
        });
        ids.sort();
        let mut expected = (0..32).map(|n| format!("task_{n}")).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(ids, expected);
        assert!(!queue.has_pending(task.session_id));
    }

    #[test]
    fn draining_releases_per_session_storage_and_allows_reenqueue() {
        let queue = SessionWakeQueue::new();
        let task = task_with_policy(TaskWakePolicy::OnTerminal);
        queue.note_transition(&task, TaskTransition::Terminal);
        assert_eq!(queue.drain(task.session_id).len(), 1);
        assert!(
            queue.queues.lock().unwrap().is_empty(),
            "drained sessions must not accumulate in the queue"
        );
        assert!(queue.note_transition(&task, TaskTransition::Terminal));
        assert_eq!(queue.drain(task.session_id).len(), 1);
        assert!(queue.queues.lock().unwrap().is_empty());
    }
}
