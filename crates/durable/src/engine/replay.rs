//! Deciding which replayed workflow actions still need to be applied.

use crate::workflow::{WorkflowAction, WorkflowEvent};

/// Identity of the history event an action records, used to tell which
/// replayed actions have already been applied.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RecordedAction {
    Activity(String),
    Timer(String),
    CancelActivity(String),
    ChildWorkflow(String),
    Completed,
    Failed,
}

impl RecordedAction {
    fn from_action(action: &WorkflowAction) -> Option<Self> {
        match action {
            WorkflowAction::ScheduleActivity { activity_id, .. } => {
                Some(Self::Activity(activity_id.clone()))
            }
            WorkflowAction::StartTimer { timer_id, .. } => Some(Self::Timer(timer_id.clone())),
            WorkflowAction::CancelActivity { activity_id } => {
                Some(Self::CancelActivity(activity_id.clone()))
            }
            WorkflowAction::ScheduleChildWorkflow { workflow_type, .. } => {
                Some(Self::ChildWorkflow(workflow_type.clone()))
            }
            WorkflowAction::CompleteWorkflow { .. } => Some(Self::Completed),
            WorkflowAction::FailWorkflow { .. } => Some(Self::Failed),
            WorkflowAction::None => None,
        }
    }

    fn from_event(event: &WorkflowEvent) -> Option<Self> {
        match event {
            WorkflowEvent::ActivityScheduled { activity_id, .. } => {
                Some(Self::Activity(activity_id.clone()))
            }
            WorkflowEvent::TimerStarted { timer_id, .. } => Some(Self::Timer(timer_id.clone())),
            WorkflowEvent::ActivityCancelled { activity_id, .. } => {
                Some(Self::CancelActivity(activity_id.clone()))
            }
            WorkflowEvent::ChildWorkflowStarted { workflow_type, .. } => {
                Some(Self::ChildWorkflow(workflow_type.clone()))
            }
            WorkflowEvent::WorkflowCompleted { .. } => Some(Self::Completed),
            WorkflowEvent::WorkflowFailed { .. } => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Return the replayed actions whose events are not yet in `history`.
///
/// Decision: replay is deterministic, so the workflow re-issues every action
/// it ever requested, in order. Each action that already has its recording
/// event in history consumes one occurrence of that event; whatever is left
/// was requested but never applied. That covers the normal case (the follow-up
/// to the activity completion just appended) and a crash between appending an
/// event and applying its actions, so `process_workflow` is safe to re-run.
/// Counting occurrences rather than ids lets a workflow reuse an activity id.
pub(super) fn unrecorded_actions<'a>(
    actions: Vec<WorkflowAction>,
    history: impl IntoIterator<Item = &'a WorkflowEvent>,
) -> Vec<WorkflowAction> {
    let mut recorded: std::collections::HashMap<RecordedAction, usize> =
        std::collections::HashMap::new();
    for key in history.into_iter().filter_map(RecordedAction::from_event) {
        *recorded.entry(key).or_default() += 1;
    }

    actions
        .into_iter()
        .filter(|action| {
            let Some(key) = RecordedAction::from_action(action) else {
                return false;
            };
            match recorded.get_mut(&key) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    false
                }
                _ => true,
            }
        })
        .collect()
}

pub(super) fn has_terminal_action(actions: &[WorkflowAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            WorkflowAction::CompleteWorkflow { .. } | WorkflowAction::FailWorkflow { .. }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_unrecorded_actions_counts_reused_activity_ids() {
        let schedule = |id: &str| WorkflowAction::schedule_activity(id, "t", serde_json::json!({}));
        let scheduled = |id: &str| WorkflowEvent::ActivityScheduled {
            activity_id: id.into(),
            activity_type: "t".into(),
            input: serde_json::json!({}),
            options: Default::default(),
        };

        // "a" was scheduled once already; a retry of "a" plus a new "b" remain.
        let history = [scheduled("a")];
        let pending = unrecorded_actions(
            vec![
                schedule("a"),
                schedule("a"),
                schedule("b"),
                WorkflowAction::None,
            ],
            history.iter(),
        );

        assert_eq!(pending, vec![schedule("a"), schedule("b")]);
    }

    #[test]
    fn test_unrecorded_actions_matches_each_action_kind() {
        let history = [
            WorkflowEvent::TimerStarted {
                timer_id: "t1".into(),
                duration_ms: 10,
            },
            WorkflowEvent::ActivityCancelled {
                activity_id: "a1".into(),
                reason: "x".into(),
            },
            WorkflowEvent::ChildWorkflowStarted {
                workflow_id: Uuid::now_v7(),
                workflow_type: "child".into(),
            },
            WorkflowEvent::WorkflowFailed {
                error: crate::WorkflowError::new("boom"),
            },
        ];
        let recorded = vec![
            WorkflowAction::timer("t1", std::time::Duration::from_millis(10)),
            WorkflowAction::CancelActivity {
                activity_id: "a1".into(),
            },
            WorkflowAction::ScheduleChildWorkflow {
                workflow_id: "c1".into(),
                workflow_type: "child".into(),
                input: serde_json::json!({}),
            },
            WorkflowAction::fail(crate::WorkflowError::new("boom")),
        ];
        assert!(unrecorded_actions(recorded, history.iter()).is_empty());

        let complete = WorkflowAction::complete(serde_json::json!(1));
        assert_eq!(
            unrecorded_actions(vec![complete.clone()], history.iter()),
            vec![complete]
        );
    }
}
