//! Durable Astra effort snapshots and their ordered request projections.
//!
//! Snapshots live on completed assistant events, so the immutable transcript is
//! the source of truth across worker restarts. Checkpoints retain the same state
//! when they replace that transcript. Request retries reuse the prepared state.

use crate::message::{Message, MessageRole};
use crate::typed_id::MessageId;
use everruns_provider::ReasoningEffort;
use everruns_provider::reasoning_updates::{
    ReasoningState, supported_update_effort, supports_configuration_updates,
};
use std::collections::HashMap;

pub(super) const STATE_KEY: &str = "openai_reasoning_state";

pub(super) struct ReasoningReplay {
    pub state: ReasoningState,
    pub transitions: HashMap<MessageId, ReasoningEffort>,
    pub reset_continuation: bool,
}

fn snapshot(message: &Message) -> Option<ReasoningState> {
    if message.role != MessageRole::Agent {
        return None;
    }
    let metadata = message.metadata.as_ref()?;
    if metadata.get("provider")?.as_str()? != "openai"
        || metadata.get("model")?.as_str()? != "gpt-6-astra"
    {
        return None;
    }
    serde_json::from_value::<ReasoningState>(metadata.get(STATE_KEY)?.clone())
        .ok()
        .filter(ReasoningState::is_supported)
}

pub(super) fn prepare(
    messages: &[Message],
    provider: &str,
    model: &str,
    requested: Option<ReasoningEffort>,
    live: Option<ReasoningEffort>,
    checkpoint: Option<&ReasoningState>,
) -> Option<ReasoningReplay> {
    if provider != "openai"
        || !supports_configuration_updates(model)
        || requested.is_some_and(|effort| !supported_update_effort(effort))
        || live.is_some_and(|effort| !supported_update_effort(effort))
    {
        return None;
    }
    let messages: Vec<_> = messages
        .iter()
        .filter(|message| !super::is_error_placeholder_message(message))
        .collect();
    let last_assistant = messages
        .iter()
        .rposition(|message| message.role == MessageRole::Agent);
    let prior = last_assistant
        .and_then(|index| snapshot(messages[index]))
        .or_else(|| {
            if last_assistant.is_none() {
                checkpoint.cloned()
            } else {
                None
            }
        });
    let reset_continuation = prior.is_none();
    let mut state = prior.unwrap_or_else(|| ReasoningState {
        epoch: uuid::Uuid::now_v7().to_string(),
        baseline: requested,
        effective: requested,
        pending: None,
    });
    let latest_new_user_effort = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(index, message)| {
            message.role == MessageRole::User && last_assistant.is_none_or(|last| *index > last)
        })
        .and_then(|(_, message)| message.controls.as_ref())
        .and_then(|controls| controls.reasoning.as_ref())
        .and_then(|reasoning| reasoning.effort);
    if latest_new_user_effort.is_some_and(|effort| !supported_update_effort(effort)) {
        return None;
    }
    let desired = live
        .filter(|effort| supported_update_effort(*effort))
        .or(latest_new_user_effort)
        .or(state.effective);
    state.pending = (desired != state.effective).then_some(desired).flatten();
    state.effective = desired;

    let mut transitions = HashMap::new();
    let mut effort = checkpoint
        .filter(|checkpoint| checkpoint.epoch == state.epoch)
        .and_then(|checkpoint| checkpoint.effective)
        .or(state.baseline);
    let mut input_start = 0;
    for (index, message) in messages.iter().enumerate() {
        if message.role != MessageRole::Agent {
            continue;
        }
        if let Some(saved) = snapshot(message).filter(|saved| saved.epoch == state.epoch) {
            if let Some(next) = saved.effective.filter(|next| Some(*next) != effort) {
                // A pure resume can have no new input: in that case the update
                // directly precedes the assistant output it configured.
                let anchor = messages[input_start..=index]
                    .iter()
                    .find(|message| message.role != MessageRole::System)
                    .unwrap_or(message);
                transitions.insert(anchor.id, next);
            }
            effort = saved.effective;
        }
        input_start = index + 1;
    }
    Some(ReasoningReplay {
        state,
        transitions,
        reset_continuation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReasoningEffort::{High, Low, Max, Minimal};

    fn user(effort: Option<ReasoningEffort>) -> Message {
        let mut message = Message::user("continue");
        message.controls = Some(crate::message::Controls {
            reasoning: Some(crate::message::ReasoningConfig { effort }),
            ..Default::default()
        });
        message
    }

    fn completed(state: &ReasoningState) -> Message {
        let mut message = Message::assistant("done");
        message.metadata = Some(HashMap::from([
            ("provider".into(), serde_json::json!("openai")),
            ("model".into(), serde_json::json!("gpt-6-astra")),
            (STATE_KEY.into(), serde_json::json!(state)),
        ]));
        // Exercise the exact persisted representation; pending must not survive.
        serde_json::from_value(serde_json::to_value(message).unwrap()).unwrap()
    }

    fn plan(
        messages: &[Message],
        requested: Option<ReasoningEffort>,
        live: Option<ReasoningEffort>,
    ) -> ReasoningReplay {
        prepare(messages, "openai", "gpt-6-astra", requested, live, None).unwrap()
    }

    #[test]
    fn restart_retains_baseline_effective_effort_and_ordered_transitions() {
        let mut messages = vec![user(Some(Low))];
        let first = plan(&messages, Some(Low), None);
        assert_eq!(first.state.baseline, Some(Low));
        assert_eq!(first.state.pending, None);
        messages.push(completed(&first.state));
        let followup = user(Some(High));
        let anchor = followup.id;
        messages.push(followup);
        let high = plan(&messages, Some(High), None);
        assert_eq!(high.state.baseline, Some(Low));
        assert_eq!(high.state.pending, Some(High));
        messages.push(completed(&high.state));
        // A fresh worker has no live handle, and the old user's low/high
        // controls must not override the last successfully used effort.
        let restored = plan(&messages, Some(High), None);
        assert_eq!(restored.state.baseline, Some(Low));
        assert_eq!(restored.state.effective, Some(High));
        assert_eq!(restored.state.pending, None);
        assert_eq!(restored.transitions, HashMap::from([(anchor, High)]));
        let max = plan(&messages, Some(High), Some(Max));
        assert_eq!(max.state.pending, Some(Max));
        messages.push(completed(&max.state));
        let restored = plan(&messages, Some(High), None);
        assert_eq!(restored.state.effective, Some(Max));
        assert_eq!(restored.state.pending, None);
    }

    #[test]
    fn repeated_live_updates_collapse_and_returning_to_baseline_is_a_transition() {
        let first = plan(&[user(Some(Low))], Some(Low), None);
        let messages = vec![user(Some(Low)), completed(&first.state)];
        let changed = plan(&messages, Some(Low), Some(High));
        let mut messages = messages;
        messages.push(completed(&changed.state));
        assert_eq!(plan(&messages, Some(Low), Some(High)).state.pending, None);
        assert_eq!(
            plan(&messages, Some(Low), Some(Low)).state.pending,
            Some(Low)
        );
    }

    #[test]
    fn unset_baseline_stays_unset_when_later_effort_is_selected() {
        let first = plan(&[user(None)], None, None);
        let messages = vec![user(None), completed(&first.state), user(Some(High))];
        let next = plan(&messages, Some(High), None);
        assert_eq!(next.state.baseline, None);
        assert_eq!(next.state.pending, Some(High));
    }

    #[test]
    fn incompatible_models_and_efforts_do_not_replay_configuration() {
        for (provider, model, effort) in [
            ("openrouter", "gpt-6-astra", High),
            ("openai", "gpt-6-astra-pro", High),
            ("openai", "gpt-5.4", High),
            ("openai", "gpt-6-astra", Minimal),
        ] {
            assert!(prepare(&[], provider, model, Some(effort), None, None).is_none());
        }
        let first = plan(&[user(Some(Low))], Some(Low), None);
        let mut foreign = completed(&first.state);
        foreign
            .metadata
            .as_mut()
            .unwrap()
            .insert("model".into(), serde_json::json!("gpt-5.4"));
        let messages = vec![completed(&first.state), foreign, user(Some(High))];
        let switched = plan(&messages, Some(High), None);
        assert!(switched.reset_continuation);
        assert_ne!(switched.state.epoch, first.state.epoch);
        assert_eq!(switched.state.baseline, Some(High));
        assert!(switched.transitions.is_empty());
    }

    #[test]
    fn checkpoint_restores_baseline_without_replaying_consumed_transitions() {
        let checkpoint = ReasoningState {
            epoch: "epoch".into(),
            baseline: Some(Low),
            effective: Some(High),
            pending: None,
        };
        let restored = prepare(
            &[user(None)],
            "openai",
            "gpt-6-astra",
            None,
            None,
            Some(&checkpoint),
        )
        .unwrap();
        assert_eq!(restored.state, checkpoint);
        assert!(restored.transitions.is_empty());
        let changed = prepare(
            &[user(Some(Max))],
            "openai",
            "gpt-6-astra",
            Some(Max),
            None,
            Some(&checkpoint),
        )
        .unwrap();
        assert_eq!(changed.state.baseline, Some(Low));
        assert_eq!(changed.state.pending, Some(Max));
    }
}
