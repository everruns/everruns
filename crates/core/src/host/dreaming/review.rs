use super::*;

impl<A: RuntimeHostAdapter> RuntimeDreamingHost<A> {
    pub async fn proposals(&self, accept: bool, discard: bool) -> Result<String> {
        let session = self.assembled.session_id();
        let key = job_key(session);
        let Some(mut row) = self.store.get(self.org, &key).await? else {
            return Ok("No pending dreaming proposals.".into());
        };
        let mut state: DreamSessionState = serde_json::from_str(&row.value)
            .map_err(|_| AgentLoopError::store("invalid dreaming state"))?;
        let Some(job) = state.job.as_mut() else {
            return Ok("No pending dreaming proposals.".into());
        };
        if !job.completed {
            return Ok(
                "Dreaming is still processing destinations; review after completion.".into(),
            );
        }
        if job.proposals.is_empty() {
            return Ok("No pending dreaming proposals.".into());
        }
        if discard && job.review_accepting {
            return Err(AgentLoopError::config(
                "acceptance is in progress; retry accept to finish",
            ));
        }
        if accept && !job.review_accepting {
            job.review_accepting = true;
            let value = serde_json::to_string(&state)
                .map_err(|_| AgentLoopError::store("serialize review claim"))?;
            if !self
                .store
                .compare_exchange(
                    self.org,
                    &key,
                    Some(row.revision),
                    &value,
                    Some(session),
                    None,
                )
                .await?
            {
                return Err(AgentLoopError::store(
                    "dreaming review changed concurrently; retry",
                ));
            }
            row.revision += 1;
        }
        let job = state
            .job
            .as_mut()
            .ok_or_else(|| AgentLoopError::store("missing review job"))?;
        let mut display = String::new();
        for (index, output) in &job.proposals {
            let d = job
                .config
                .destinations
                .get(*index)
                .ok_or_else(|| AgentLoopError::config("invalid proposal destination"))?;
            self.destination(session, d).await?;
            for note in &output.notes {
                display.push_str(&format!(
                    "\n- **{}:** {}\n  Sources: {}\n",
                    note.kind,
                    note.text,
                    note.evidence.join(", ")
                ));
            }
            if accept {
                let mut approved = job.clone();
                approved.config.commit_mode = DreamCommitMode::Automatic;
                approved.destination_index = *index;
                self.commit(&approved, d, output).await?;
            }
        }
        if accept || discard {
            let operation_id = job.operation_id.clone();
            let note_count = job
                .proposals
                .values()
                .map(|output| output.notes.len())
                .sum();
            job.proposals.clear();
            if !self
                .store
                .compare_exchange(
                    self.org,
                    &key,
                    Some(row.revision),
                    &serde_json::to_string(&state)
                        .map_err(|_| AgentLoopError::store("serialize review"))?,
                    Some(session),
                    None,
                )
                .await?
            {
                return Err(AgentLoopError::store(
                    "dreaming review changed concurrently; retry",
                ));
            }
            self.activity(
                session,
                DreamActivity {
                    operation_id,
                    status: if accept { "accepted" } else { "discarded" }.into(),
                    destination: None,
                    note_count,
                    reason: None,
                    cost_usd: None,
                    model: None,
                },
            )
            .await?;
            return Ok(if accept {
                "Dreaming proposals accepted."
            } else {
                "Dreaming proposals discarded."
            }
            .into());
        }
        Ok(display)
    }
}
