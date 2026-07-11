// Embeddable in-process task-transition observer (EVE-729).
//
// A `TaskTransition` names a lifecycle transition a `SessionTaskRegistry` fires
// on: reaching a terminal state, entering `awaiting_input`, or emitting an
// outbound message. A `TaskTransitionObserver` receives those transitions in
// process — the same seam the server's webhook dispatcher uses, minus HTTP.
//
// Design Decision: the enum + trait live in `everruns-core` (not the server) so
// `everruns-runtime` embedders can observe task transitions without depending on
// the control-plane server or making HTTP calls. The server webhook dispatcher
// (`DirectTaskWebhookNotifier`) is one implementation of this trait; in-process
// embedders provide their own. A `SessionTaskRegistry` fires each real
// transition once to every registered observer, so an in-process observer sees
// exactly the same transitions the webhook path fires (see the parity test in
// `crates/server/src/storage/session_task_store.rs`).
//
// Filter semantics: `Terminal` is the regression-safe default (org webhooks only
// ever fire on it); `AwaitingInput` and `Message` are the non-terminal
// transitions that are opt-in per delivery target via `event_filter` (EVE-682).
// The `filter_value` / `event_name` strings are shared with webhook payloads so
// the two paths stay byte-for-byte aligned.

use async_trait::async_trait;

use crate::session_task::SessionTask;

/// A task lifecycle transition an observer can be notified of.
///
/// `Terminal` is the only transition org webhooks ever fire on (regression-safe).
/// `AwaitingInput` and `Message` are opt-in per delivery target via
/// `event_filter` (EVE-682).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskTransition {
    /// Task reached a terminal state (succeeded / failed / canceled).
    Terminal,
    /// Task transitioned into `awaiting_input`.
    AwaitingInput,
    /// Task emitted an outbound message.
    Message,
}

impl TaskTransition {
    /// The `event_filter` member string that enables this transition.
    pub fn filter_value(&self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::AwaitingInput => "awaiting_input",
            Self::Message => "message",
        }
    }

    /// The `event` field value in a delivered webhook payload.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Terminal => "task.terminal",
            Self::AwaitingInput => "task.awaiting_input",
            Self::Message => "task.message",
        }
    }
}

/// Receive task-transition notifications in process.
///
/// A `SessionTaskRegistry` invokes `on_transition` once per real transition for
/// every registered observer. Implementations must treat delivery as
/// best-effort: the registry logs errors and never fails the underlying task
/// operation because an observer returned `Err`. Observers must not block for
/// long — the registry dispatches them off the task-update path, but a slow
/// observer still delays its own delivery.
///
/// The server webhook dispatcher (`DirectTaskWebhookNotifier`) is one
/// implementation. Embedders of `everruns-runtime` implement this trait to get
/// in-process callbacks with the same transition semantics, without HTTP.
#[async_trait]
pub trait TaskTransitionObserver: Send + Sync + 'static {
    /// Handle one task transition. Best-effort: returning `Err` is logged and
    /// never fails the task operation that produced the transition.
    async fn on_transition(
        &self,
        task: &SessionTask,
        transition: TaskTransition,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_value_and_event_name_are_stable() {
        // These strings are a wire contract shared with webhook payloads and the
        // per-task `event_filter`; changing them silently breaks delivery.
        assert_eq!(TaskTransition::Terminal.filter_value(), "terminal");
        assert_eq!(
            TaskTransition::AwaitingInput.filter_value(),
            "awaiting_input"
        );
        assert_eq!(TaskTransition::Message.filter_value(), "message");

        assert_eq!(TaskTransition::Terminal.event_name(), "task.terminal");
        assert_eq!(
            TaskTransition::AwaitingInput.event_name(),
            "task.awaiting_input"
        );
        assert_eq!(TaskTransition::Message.event_name(), "task.message");
    }
}
