//! One-transaction import of a quiescent legacy session (EVE-1298).
//!
//! The whole image commits under one IMMEDIATE transaction or not at all, so
//! an interrupted import leaves no partial authority state to repair. The
//! committed cutover record makes a retry replay and a different cutover fail.
use super::*;
use crate::hosting::cutover::{CUTOVER_RECORD, CUTOVER_WORKSPACE, LegacySessionImage};
use crate::hosting::spi::session_commands::{self, CommandView, SessionCommand};

impl SqliteHosting {
    pub(super) fn import_image(
        &self,
        session: SessionId,
        image: LegacySessionImage,
    ) -> Result<bool> {
        let decision = image.record();
        // Seal outside the transaction; refuse before any write without keys.
        let sealed = image
            .secrets
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.clone(),
                    self.secret_keys()?.seal(session, name, value)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        self.transaction(session, |tx, state| {
            if state.binding.is_some() {
                let committed: Option<Value> = object(tx, session, "framework", CUTOVER_RECORD)?;
                return match committed {
                    Some(mut committed) => {
                        if let Some(record) = committed.as_object_mut() {
                            record.remove("decided_at");
                        }
                        if committed == decision {
                            Ok(true)
                        } else {
                            Err(AgentLoopError::config(
                                "a different cutover already owns this session",
                            ))
                        }
                    }
                    None => Err(AgentLoopError::config(
                        "the authority already holds this session outside a cutover",
                    )),
                };
            }
            // Creation rules (binding validation) stay in the shared reducer.
            let binding = json!(image.binding);
            let plan = session_commands::prepare(
                SessionCommand::Create {
                    binding: binding.clone(),
                },
                CommandView::new(session, now() as i64),
            )?;
            if !matches!(plan.control, session_commands::ControlUpdate::Bind { .. }) {
                return Err(error("unexpected creation plan"));
            }
            state.binding = Some(image.binding.clone());
            state.created_at = everruns_contracts::rt::utc_now_iso();
            import_journal(tx, session, state, &image)?;
            import_objects(tx, session, state, &image)?;
            for (name, secret) in &sealed {
                put(tx, session, "secret", name, secret)?;
            }
            let mut record = decision.clone();
            record["decided_at"] = json!(everruns_contracts::rt::utc_now_iso());
            put(tx, session, "framework", CUTOVER_RECORD, &record)?;
            if let Some(workspace) = &image.workspace {
                put(tx, session, "framework", CUTOVER_WORKSPACE, workspace)?;
            }
            Ok(false)
        })
    }
}

fn import_journal(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &mut Control,
    image: &LegacySessionImage,
) -> Result<()> {
    let mut last_input = 0i64;
    for event in &image.events {
        let sequence = event
            .sequence
            .filter(|sequence| *sequence > state.head)
            .ok_or_else(|| error("legacy journal sequence is not strictly increasing"))?;
        if event.session_id != session {
            return Err(AgentLoopError::config(
                "legacy event belongs to another session",
            ));
        }
        let encoded = encode(event)?;
        if encoded.len() > 2 * 1024 * 1024 {
            return Err(error("legacy event exceeds the authority size limit"));
        }
        let turn = event.context.turn_id;
        // A legacy log may repeat an outcome (the facade lane did not
        // deduplicate). Every event is kept; only the first carries the index.
        let identity = event_identity(&event.data, turn)?.filter(|identity| {
            !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM framework_events WHERE session=? AND identity=?)",
                params![session.to_string(), identity],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(true)
        });
        tx.execute(
            "INSERT INTO framework_events VALUES (?,?,?,?,?,?)",
            params![
                session.to_string(),
                sequence,
                event.event_type,
                turn.map(|id| id.to_string()),
                identity,
                encoded
            ],
        )
        .map_err(error)?;
        state.head = sequence;
        if matches!(event.data, EventData::InputMessage(_)) {
            last_input = i64::from(sequence);
        }
        advance(state, event);
    }
    // Imported turns finished their Framework lifecycle on the legacy lane;
    // their outcomes are written below. Every imported input is consumed: an
    // input the legacy executor never ran is history, not work this cutover
    // starts on its own. An interrupted open turn stays open, so the shared
    // driver applies its ordinary recovery (no rerun of uncertain effects)
    // when the session next runs.
    state.finalizing.clear();
    state.inbox = last_input;
    state.next_input = last_input;
    state.consumed = i64::from(state.head);
    state.runnable = false;
    // An imported approval wait keeps its deadline as an independent wakeup;
    // only the shared execution path interprets the expiry.
    state.wake_at = state
        .pending
        .iter()
        .filter_map(crate::hosting::execution::approval_deadline)
        .min();
    Ok(())
}

fn import_objects(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &Control,
    image: &LegacySessionImage,
) -> Result<()> {
    if let Some(record) = &image.record {
        put(tx, session, "session", "record", record)?;
    }
    if json!(image.configuration) != json!(SessionConfiguration::default()) {
        put(
            tx,
            session,
            "framework",
            "configuration",
            &image.configuration,
        )?;
    }
    for checkpoint in &image.checkpoints {
        if checkpoint.session_id != session || checkpoint.source_sequence > i64::from(state.head) {
            return Err(error("legacy checkpoint exceeds the imported journal"));
        }
        let key = json!([
            checkpoint.provider_type,
            checkpoint.model,
            checkpoint.format_version
        ])
        .to_string();
        put(
            tx,
            session,
            "checkpoint",
            &key,
            &json!({
                "id": checkpoint.id,
                "session_id": checkpoint.session_id,
                "source_sequence": checkpoint.source_sequence,
                "provider_type": checkpoint.provider_type,
                "model": checkpoint.model,
                "format_version": checkpoint.format_version,
                "payload": checkpoint.payload,
            }),
        )?;
    }
    for (key, value) in &image.values {
        put(tx, session, "value", key, value)?;
    }
    // Tasks land in the authority's task registry tables, in the same
    // transaction; running ones are reconciled by the next activation.
    for (task, messages) in &image.tasks {
        if task.session_id != session {
            return Err(AgentLoopError::config(
                "legacy task belongs to another session",
            ));
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM local_tasks WHERE id=?)",
                [&task.id],
                |row| row.get(0),
            )
            .map_err(error)?;
        if exists {
            return Err(AgentLoopError::config(
                "a legacy task id already exists in the authority",
            ));
        }
        crate::local::LocalSessionTaskRegistry::store_task(tx, task)?;
        for message in messages {
            crate::local::LocalSessionTaskRegistry::store_message(tx, message)?;
        }
    }
    import_admissions(tx, session, image)
}

/// Admission receipts, turn assignments and Framework outcomes, so admission
/// handles of legacy inputs resolve and a retried legacy send replays.
fn import_admissions(
    tx: &Transaction<'_>,
    session: SessionId,
    image: &LegacySessionImage,
) -> Result<()> {
    let mut turns: HashMap<MessageId, TurnId> = HashMap::new();
    for event in &image.events {
        if let EventData::TurnStarted(data) = &event.data {
            turns.insert(data.input_message_id, data.turn_id);
            put(
                tx,
                session,
                "framework",
                &format!("turn_for/{}", data.input_message_id),
                &data.turn_id,
            )?;
        }
    }
    for event in &image.events {
        let EventData::InputMessage(data) = &event.data else {
            continue;
        };
        let message = data.message.clone();
        let id = message.id;
        let sequence = event.sequence.unwrap_or_default();
        let command = SessionCommand::Admit {
            message: Box::new(message),
            cancelled: false,
        };
        let seen: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM framework_commands WHERE session=? AND id=?)",
                params![session.to_string(), id.to_string()],
                |row| row.get(0),
            )
            .map_err(error)?;
        if !seen {
            retain_command(
                tx,
                session,
                &id.to_string(),
                &command.fingerprint(),
                &json!({"message_id": id, "sequence": sequence}),
            )?;
        }
        let turn = event.context.turn_id.or_else(|| turns.get(&id).copied());
        if let Some(turn) = turn {
            let steered = turns.get(&id) != Some(&turn);
            put(
                tx,
                session,
                "framework",
                &format!("assignment/{id}"),
                &json!({"turn_id": turn, "steered": steered}),
            )?;
        }
    }
    let mut finished = HashSet::new();
    for event in &image.events {
        if is_terminal(&event.data)
            && let Some(turn) = event.context.turn_id
            && finished.insert(turn)
            && let Some(outcome) = crate::hosting::execution::project_terminal(
                &image
                    .events
                    .iter()
                    .filter(|candidate| candidate.context.turn_id == Some(turn))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        {
            put(
                tx,
                session,
                "framework",
                &format!("outcome/{turn}"),
                &outcome,
            )?;
        }
    }
    Ok(())
}
