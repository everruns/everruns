//! Authority-backed AG-UI observation and interruption.
use super::*;

pub(super) async fn next_event(mut state: RunState) -> Option<(Event, RunState)> {
    loop {
        if let Some(event) = state.projector.pop() {
            return Some((event, state));
        }
        if state.projector.is_finished() {
            return None;
        }
        let session_id = state.session.session_id();
        tokio::select! {
            () = async {
                if state.session.has_hosted_execution() {everruns_contracts::rt::sleep(std::time::Duration::from_millis(50)).await;} else {std::future::pending().await}
            } => {
                if let Err(error)=state.session.refresh_hosted_calls().await {state.projector.fail(RunErrorEvent::new(error.to_string()));}
                state.interrupt_if_parked().await;
            },
            next = state.events.recv() => match next {
                Ok(Some(event)) => state.project(&event),
                Ok(None) => state.projector.fail(RunErrorEvent::new(
                    "the session closed before the run finished",
                )),
                Err(EventStreamError::Lagged { .. }) => state.recover().await,
                Err(EventStreamError::Unavailable) => state.projector.fail(RunErrorEvent::new("session event authority unavailable")),
            },
            () = parked_on(&mut state.parked, session_id) => state.interrupt_if_parked().await,
            () = parked_with(&mut state.parked_calls, state.awaiting.as_deref()) => {
                state.park_frontend_calls().await;
            }
        }
    }
}

/// Resolves once the session has parked on every call in `awaiting`, which
/// it records as the turn ends. Pending while nothing is awaited.
async fn parked_with(
    parked: &mut watch::Receiver<Option<ParkedToolCalls>>,
    awaiting: Option<&[WireToolCall]>,
) {
    let Some(calls) = awaiting else {
        return std::future::pending().await;
    };
    let covered = parked.wait_for(|parked| {
        parked.as_ref().is_some_and(|parked| {
            calls
                .iter()
                .all(|call| parked.tool_calls.iter().any(|made| made.id == call.id))
        })
    });
    if covered.await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Resolves when a request parks on `session_id`, or the gate's notices
/// lagged (the open interrupts are re-read either way).
async fn parked_on(parked: &mut Option<broadcast::Receiver<SessionId>>, session_id: SessionId) {
    let Some(rx) = parked else {
        return std::future::pending().await;
    };
    loop {
        match rx.recv().await {
            Ok(id) if id == session_id => return,
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Lagged(_)) => return,
            Err(broadcast::error::RecvError::Closed) => return std::future::pending().await,
        }
    }
}

impl RunState {
    fn project(&mut self, event: &crate::SessionEvent) {
        if let Some(sequence) = event.sequence() {
            self.last_sequence = Some(self.last_sequence.map_or(sequence, |s| s.max(sequence)));
        }
        let data = event
            .canonical_json()
            .get("data")
            .cloned()
            .unwrap_or(Value::Null);
        // Calls to this run's frontend tools are the consumer's to run: the
        // turn parks on them and the run ends naming them, once it has.
        if event.event_type() == "tool.call_requested" {
            let calls: Vec<WireToolCall> = serde_json::from_value::<ToolCallRequestedData>(data)
                .map(|requested| {
                    requested
                        .tool_calls
                        .into_iter()
                        .filter(|call| self.frontend.contains(&call.name))
                        .map(|call| {
                            WireToolCall::function(call.id, call.name, call.arguments.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !calls.is_empty() {
                self.awaiting = Some(calls);
            }
            return;
        }
        self.projector.project(event.event_type(), &data);
    }

    /// The turn parked on the frontend calls it requested: flush what it
    /// emitted, then end the run with them, beside any open interrupt.
    async fn park_frontend_calls(&mut self) {
        let Some(calls) = self.awaiting.take() else {
            return;
        };
        while let Ok(Some(event)) = self.events.try_recv() {
            self.project(&event);
            if self.projector.is_finished() {
                return;
            }
        }
        let interrupts = match &self.gate {
            Some(gate) => match gate.query(self.session.session_id()).await {
                Ok(interrupts) => interrupts,
                Err(error) => {
                    self.projector.fail(RunErrorEvent::new(error.to_string()));
                    return;
                }
            },
            None => Vec::new(),
        };
        self.projector.park(calls, interrupts);
    }

    /// The run fell behind the live feed: replay the durable log after the
    /// last durable event seen. Ephemeral deltas in the gap are lost, which
    /// only shortens streamed text; completions are durable.
    async fn recover(&mut self) {
        let Some(after) = self.last_sequence else {
            self.projector.fail(RunErrorEvent::new(
                "the run fell behind the session's event stream",
            ));
            return;
        };
        match self.session.events_from(after).await {
            Ok(events) => self.events = events,
            Err(error) => self.projector.fail(RunErrorEvent::new(format!(
                "the run fell behind the session's event stream: {error}"
            ))),
        }
    }

    /// A request parked: flush what the turn already emitted, then end the
    /// run with every open interrupt.
    async fn interrupt_if_parked(&mut self) {
        let Some(gate) = &self.gate else { return };
        let interrupts = match gate.query(self.session.session_id()).await {
            Ok(interrupts) => interrupts,
            Err(error) => {
                self.projector.fail(RunErrorEvent::new(error.to_string()));
                return;
            }
        };
        if interrupts.is_empty() {
            return;
        }
        while let Ok(Some(event)) = self.events.try_recv() {
            self.project(&event);
            if self.projector.is_finished() {
                return;
            }
        }
        self.projector.interrupt(interrupts);
    }
}
