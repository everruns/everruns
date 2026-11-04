//! Model-switch observations kept separate from turn lifecycle orchestration.
use super::*;

/// Emit `session.model.changed` when this turn's input selects a model
/// different from the previous turn's.
///
/// Best effort: a missing marker must not fail the turn.
pub(super) async fn emit_model_change_if_switched<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    input: &ReasonInput,
    assembled: &AssembledTurnContext,
) {
    let Some((previous_model_id, model_id)) = model_switch(&assembled.messages) else {
        return;
    };
    if assembled.resolved_model_id != Some(model_id) {
        // The requested model did not survive resolution (unknown or removed),
        // so the turn runs on a fallback this marker would misname.
        return;
    }

    // The previous model is named by looking it up; the current one is already
    // resolved for this turn. Names are captured now so the transcript stays
    // readable after a model is removed from the org.
    let previous_model_name = adapter
        .provider_store(org_id)
        .get_model_spec(previous_model_id)
        .await
        .ok()
        .flatten()
        .map(|spec| spec.model);

    let request = EventRequest::new(
        input.context.session_id,
        EventContext::turn(input.context.turn_id, input.context.input_message_id),
        SessionModelChangedData {
            previous_model_id: Some(previous_model_id),
            previous_model_name,
            model_id,
            model_name: assembled.model.model.clone(),
        },
    );
    if let Err(e) = adapter.event_emitter().emit(request).await {
        warn!(error = %e, "Failed to emit session.model.changed event");
    }
}

/// `(previous, current)` model overrides when the turn's input switched models.
///
/// Only an explicit override replacing a different explicit override counts. A
/// turn without an override runs on an inherited default, and history visible
/// here is capability-filtered: treating a missing override as "the default"
/// would report a switch whenever an older message was filtered out.
fn model_switch(messages: &[RuntimeMessage]) -> Option<(ModelId, ModelId)> {
    let mut user_model_ids = messages
        .iter()
        .rev()
        .filter(|message| message.role == RuntimeMessageRole::User)
        .map(|message| {
            message
                .controls
                .as_ref()
                .and_then(|controls| controls.model_id)
        });

    // `latest_model_override` in `runtime_context` resolves the turn's model
    // from the last user message alone, so the comparison is between the last
    // two user messages — not the last two overrides anywhere in history.
    let model_id = user_model_ids.next().flatten()?;
    let previous_model_id = user_model_ids.next().flatten()?;
    (previous_model_id != model_id).then_some((previous_model_id, model_id))
}
