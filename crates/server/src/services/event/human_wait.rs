//! Rebuild wait facts from committed events, not process-local listener clocks.
use super::*;
use chrono::SubsecRound;
use everruns_core::human_wait::{HumanWaitState, may_resolve};

impl EventService {
    pub(super) async fn attach_human_waits(
        &self,
        request: &mut EventRequest,
        batch: &[EventRequest],
    ) -> Result<()> {
        if !may_resolve(&request.data) || request.context.turn_id.is_none() {
            return Ok(());
        }
        let mut state = HumanWaitState::default();
        let requested = self
            .db
            .list_events(
                request.session_id,
                None,
                None,
                &["tool.call_requested".into()],
                &[],
                None,
                Some(1),
            )
            .await?
            .pop();
        if let Some(requested) = requested {
            let requested = Self::row_to_event(requested);
            if requested.context.turn_id == request.context.turn_id {
                // Read the committed anchor even after the turn has terminated.
                let started = self
                    .db
                    .list_events(
                        request.session_id,
                        None,
                        None,
                        &["turn.started".into()],
                        &[],
                        requested.sequence,
                        Some(1),
                    )
                    .await?
                    .pop();
                if let Some(started) = started {
                    let started = Self::row_to_event(started);
                    if started.context.turn_id == request.context.turn_id {
                        state.observe(&started);
                    }
                }
                state.observe(&requested);
                let settled = self
                    .db
                    .list_events_advanced(&ListEventsParams {
                        session_id: request.session_id,
                        after_sequence: requested.sequence,
                        turn_id: request.context.turn_id.map(|turn| turn.to_string()),
                        filter_types: vec![
                            "tool.completed".into(),
                            "turn.completed".into(),
                            "turn.failed".into(),
                            "turn.cancelled".into(),
                            "turn.sealed".into(),
                        ],
                        limit: Some(257),
                        ..Default::default()
                    })
                    .await?;
                // A request batch has at most 256 resolutions plus its terminal.
                // Do not invent waits from an incomplete imported history.
                if settled.len() == 257 {
                    state = HumanWaitState::default();
                } else {
                    for event in settled {
                        state.observe(&Self::row_to_event(event));
                    }
                }
            }
        }
        // A worker may send request + resolution/terminal in the same atomic batch.
        for prior in batch
            .iter()
            .filter(|prior| prior.session_id == request.session_id)
        {
            let mut event = prior.clone().into_event(EventId::from_uuid(Uuid::nil()), 0);
            // Match PostgreSQL's stored microsecond precision so recovery retains
            // the same start time and request identity as this atomic batch.
            event.ts = event.ts.trunc_subsecs(6);
            state.observe(&event);
        }
        state.annotate(request);
        Ok(())
    }
}
