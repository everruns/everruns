//! Volatile transaction adapter for the shared Rust command implementation.
use super::super::spi::session_commands::{
    self, CommandReceipt, CommandView, ControlUpdate, SessionCommand,
};
use super::*;

impl MemoryHosting {
    pub(super) async fn execute_command(
        &self,
        id: SessionId,
        command: SessionCommand,
    ) -> Result<Value> {
        let session = if let SessionCommand::Create { binding } = &command {
            // Reject an invalid proposal before allocating a volatile session
            // slot. The immutable-binding check still runs under its gate.
            serde_json::from_value::<SessionBinding>(binding.clone()).map_err(core_error)?;
            session_commands::prepare(
                command.clone(),
                CommandView::new(id, everruns_contracts::rt::utc_now().timestamp_millis()),
            )?;
            let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            if sessions.len() >= 100_000 && !sessions.contains_key(&id) {
                return Err(AgentLoopError::store("memory session capacity reached"));
            }
            sessions
                .entry(id)
                .or_insert_with(|| Self::new_session(id))
                .clone()
        } else {
            self.session(id)
        };
        let mut state = session.control.lock().await;
        while state
            .pending_inputs
            .front()
            .is_some_and(|sequence| *sequence <= state.consumed)
        {
            state.pending_inputs.pop_front();
        }
        let head = session
            .log
            .read_page(EventReadRequest::new(
                id,
                EventReadLimit::new(1).map_err(core_error)?,
            ))
            .await
            .map_err(core_error)?
            .snapshot_high_watermark();
        let input = if let SessionCommand::Cancel { input, .. } = &command {
            Some(*input)
        } else {
            None
        };
        let assignment = input
            .and_then(|input| state.records.get(&format!("assignment/{input}")))
            .map(|value| {
                serde_json::from_value::<super::TurnAssignment>(value.clone()).map_err(core_error)
            })
            .transpose()?
            .map(|a| a.turn_id);
        let view = CommandView {
            session: id,
            now_ms: everruns_contracts::rt::utc_now().timestamp_millis(),
            binding: state.binding.as_ref().map(|value| json!(value)),
            execution_active: state.holder.is_some()
                && state.expires.is_some_and(|at| at > Instant::now()),
            suspended: state.suspended,
            suspension_reason: state
                .records
                .get("suspension")
                .and_then(|value| value["reason"].as_str().map(str::to_owned)),
            head,
            open: state
                .open
                .zip(state.open_input)
                .map(|(turn, input)| EventContext::turn(turn, input)),
            pending: state.pending.clone(),
            configuration: serde_json::from_value(
                state
                    .records
                    .get("configuration")
                    .cloned()
                    .unwrap_or(json!({})),
            )
            .map_err(core_error)?,
            previous: command
                .key()
                .and_then(|key| state.commands.get(&key).cloned()),
            command_count: state.commands.len(),
            queued: state.pending_inputs.len(),
            has_history: state.has_history,
            admitted: input.is_some_and(|input| state.commands.contains_key(&input.to_string())),
            assignment,
            terminal: assignment.is_some_and(|turn| state.terminal.contains_key(&turn)),
            rejected: input
                .is_some_and(|input| state.records.contains_key(&format!("rejected/{input}"))),
        };
        let plan = session_commands::prepare(command, view)?;
        if plan.replay {
            return Ok(plan.reply);
        }
        // Validate all fallible object conversions before writing the memory journal.
        let binding = if let ControlUpdate::Bind { binding } = &plan.control {
            Some(serde_json::from_value(binding.clone()).map_err(core_error)?)
        } else {
            None
        };
        for object in &plan.objects {
            if object.kind == "value" && object.value.as_str().is_none() {
                return Err(AgentLoopError::store("invalid shared value write"));
            }
        }
        for object in plan.objects {
            if object.kind == "value" {
                session
                    .services
                    .storage_store
                    .set_value(id, &object.key, object.value.as_str().unwrap_or_default())
                    .await?;
            } else {
                state.records.insert(object.key, object.value);
            }
        }
        let mut head = head;
        for event in plan.events {
            if let EventData::ToolCompleted(result) = &event.data {
                state.pending.retain(|call| call.id != result.tool_call_id);
            }
            if matches!(
                event.data,
                EventData::InputMessage(_) | EventData::OutputMessageCompleted(_)
            ) {
                state.has_history = true;
            }
            head = session
                .log
                .append(event)
                .await
                .map_err(core_error)?
                .sequence
                .unwrap_or(head);
        }
        match plan.control {
            ControlUpdate::None => {}
            ControlUpdate::Bind { .. } => {
                if state.binding.is_none() {
                    state.created_at = everruns_contracts::rt::utc_now_iso();
                }
                state.binding = binding;
            }
            ControlUpdate::Admit { message, cancelled } => {
                state.pending_inputs.push_back(i64::from(head));
                if cancelled {
                    state.target_cancels.insert(message, i64::from(head));
                }
                state.inbox = i64::from(head);
                if state.next_input <= state.consumed {
                    state.next_input = state.inbox;
                }
                state.runnable = true;
            }
            ControlUpdate::Resume => {
                state.inbox = i64::from(head);
                state.resume_generation += 1;
                state.runnable = true;
            }
            ControlUpdate::RetryExecution => {
                state.suspended = false;
                state.runnable = true;
                state.holder = None;
                state.expires = None;
            }
            ControlUpdate::Cancel { input, interrupt } => {
                let position = state.inbox;
                state.target_cancels.insert(input, position);
                if interrupt {
                    state.cancel = Some(position);
                    state.cancel_reported = false;
                    state.runnable = true;
                    session.renew_poke.notify_one();
                }
            }
        }
        if let Some(key) = plan.key {
            state.commands.insert(
                key,
                CommandReceipt {
                    fingerprint: plan.fingerprint,
                    value: plan.reply.clone(),
                },
            );
        }
        Ok(plan.reply)
    }
}
