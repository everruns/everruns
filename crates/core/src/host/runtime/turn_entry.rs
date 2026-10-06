//! Starting a turn from new or stored input: the input step, then the
//! engine-planned loop in the parent module.

use super::*;

impl InProcessRuntime {
    /// Execute one turn for an existing session.
    ///
    /// The input message is appended as the canonical `input.message` event;
    /// [`EventHistory`] derives the read projection from that one write. The
    /// turn then runs `input -> reason -> act` as planned step-by-step by
    /// [`crate::engine`] — the same planner the durable worker drives.
    pub async fn run_turn(
        &self,
        session_id: SessionId,
        input: impl Into<InputMessage>,
    ) -> Result<TurnResult> {
        self.run_steerable_turn(
            session_id,
            AcceptedTurnInput::new(input),
            TurnId::new(),
            TurnSteering::new(),
        )
        .await
    }

    /// Execute one turn while accepting additional user messages at reason
    /// boundaries.
    ///
    /// Part of the steering contract described on [`TurnSteering`].
    pub async fn run_steerable_turn(
        &self,
        session_id: SessionId,
        input: AcceptedTurnInput,
        turn_id: TurnId,
        steering: TurnSteering,
    ) -> Result<TurnResult> {
        // EVE-872: construct the canonical resolved execution snapshot before
        // turn planning. Missing or inactive records fail here, during
        // platform projection, and stored records never feed planning.
        let snapshot = self.resolved_execution_snapshot(session_id).await?;

        // The canonical input envelope is the only write. EventHistory rebuilds
        // the message projection from this accepted append.
        let input_message_id = self.persist_accepted_input(session_id, input).await?;
        self.run_turn_from(snapshot, session_id, input_message_id, turn_id, steering)
            .await
    }

    /// Execute one turn that starts from a message already in the session's
    /// log, persisted by the caller (for example with
    /// [`persist_accepted_input`](Self::persist_accepted_input), or by a host
    /// that records input through its own event store).
    ///
    /// The turn runs exactly as [`run_steerable_turn`](Self::run_steerable_turn)
    /// runs it after its one write; this method writes nothing before the
    /// input step. Steering works as there.
    ///
    /// # Errors
    ///
    /// As [`run_steerable_turn`](Self::run_steerable_turn); the input step
    /// fails when the session's log holds no message `input_message_id`.
    pub async fn run_stored_turn(
        &self,
        session_id: SessionId,
        input_message_id: MessageId,
        turn_id: TurnId,
        steering: TurnSteering,
    ) -> Result<TurnResult> {
        let snapshot = self.resolved_execution_snapshot(session_id).await?;
        self.run_turn_from(snapshot, session_id, input_message_id, turn_id, steering)
            .await
    }

    /// The turn after its input is in the log: the input step, then the
    /// engine-planned loop.
    async fn run_turn_from(
        &self,
        snapshot: ResolvedExecutionSnapshot,
        session_id: SessionId,
        input_message_id: MessageId,
        turn_id: TurnId,
        steering: TurnSteering,
    ) -> Result<TurnResult> {
        let org_id = in_process_internal_org_id(&snapshot.organization_id);

        // Engine-planned turn loop (EVE-842). Every reason-vs-act-vs-complete
        // decision comes from `everruns_core::engine`; this loop only executes the
        // host operation each plan names and performs the lifecycle effects the
        // engine returns as data. There is no second copy of the planning brain
        // in the runtime.
        let state = TurnState {
            org_id,
            session_id,
            harness_id: snapshot.harness_id,
            agent_id: snapshot.agent_id,
            input_message_id,
            turn_id: None,
            previous_response_id: None,
            iteration: 1,
            request_id: None,
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };

        let base_context = |exec: bool| {
            let context = ExecutionContext::new(session_id, turn_id, input_message_id)
                .with_workspace_id(snapshot.workspace_id);
            if exec { context.next_exec() } else { context }
        };

        // `process_input` is the turn's fixed entry step: the durable host
        // enqueues it before any planning, and it is what mints the turn id the
        // planner then carries.
        execute_input_activity(
            self,
            org_id,
            InputAtomInput {
                context: base_context(false),
            },
        )
        .await?;
        let mut execution = InProcessExecution::new(state);
        let transition = execution.advance(
            ActivityOutcome::ProcessInput {
                turn_id: Some(turn_id),
            },
            0,
            Utc::now(),
            HostFacts::default(),
        );
        crate::host::turn_strategy::perform_effects(self, org_id, session_id, transition.effects)
            .await?;
        // A new turn supersedes one parked on client-side tool calls; its
        // calls stay unanswered in history.
        self.take_parked_turn(session_id);
        self.drive_turn_plan(
            TurnDrive {
                session_id,
                org_id,
                turn_id,
                input_message_id,
                harness_id: snapshot.harness_id,
                agent_id: snapshot.agent_id,
                workspace_id: snapshot.workspace_id,
            },
            execution,
            transition.plan,
            steering,
        )
        .await
    }
}
