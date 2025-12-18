// Session workflow for agentic loop execution (M2)
// Uses Agent/Session/Messages model with Events as SSE notifications
// Now uses everruns-agent-loop for the core loop logic

use anyhow::Result;
use chrono::Utc;
use everruns_agent_loop::AgentConfig;
use everruns_contracts::events::AgUiEvent;
use everruns_storage::models::UpdateSession;
use everruns_storage::repositories::Database;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::activities::PersistEventActivity;
use crate::adapters::create_db_agent_loop_unified;

/// Session workflow orchestrating LLM calls and tool execution
///
/// This workflow now delegates the core agentic loop to everruns-agent-loop,
/// while managing session lifecycle (status updates, error handling).
pub struct SessionWorkflow {
    session_id: Uuid,
    agent_id: Uuid,
    db: Database,
    persist_activity: PersistEventActivity,
}

impl SessionWorkflow {
    pub async fn new(session_id: Uuid, agent_id: Uuid, db: Database) -> Result<Self> {
        let persist_activity = PersistEventActivity::new(db.clone());
        Ok(Self {
            session_id,
            agent_id,
            db,
            persist_activity,
        })
    }

    /// Execute the workflow using the AgentLoop abstraction
    pub async fn execute(&self) -> Result<()> {
        info!(
            session_id = %self.session_id,
            agent_id = %self.agent_id,
            "Starting session workflow"
        );

        // Update session status to running and set started_at
        self.update_session_status("running", Some(Utc::now()), None)
            .await?;

        // Load agent to get configuration
        let agent = self
            .db
            .get_agent(self.agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

        // Build AgentConfig from agent settings
        let model = agent
            .default_model_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "gpt-5.2".to_string());

        let config = AgentConfig::new(&agent.system_prompt, model)
            .with_max_iterations(10)
            .with_tools(Vec::new()); // TODO: Parse tools from agent/session

        // Check if there are messages to process
        let message_count = self.db.list_messages(self.session_id).await?.len();
        if message_count == 0 {
            warn!(
                session_id = %self.session_id,
                "No messages in session, skipping agent loop"
            );

            // Emit session finished event even if no messages
            let finished_event = AgUiEvent::session_finished(self.session_id.to_string());
            self.persist_activity
                .persist_event(self.session_id, finished_event)
                .await?;

            // Set session back to pending
            self.update_session_status("pending", None, None).await?;

            return Ok(());
        }

        info!(
            session_id = %self.session_id,
            message_count = message_count,
            model = %config.model,
            "Running agent loop"
        );

        // Create and run the agent loop with database-backed components
        // Uses UnifiedToolExecutor which supports both built-in and webhook tools
        let agent_loop = create_db_agent_loop_unified(config, self.db.clone())?;
        let result = agent_loop.run(self.session_id).await;

        match result {
            Ok(loop_result) => {
                info!(
                    session_id = %self.session_id,
                    iterations = loop_result.iterations,
                    final_messages = loop_result.messages.len(),
                    "Agent loop completed successfully"
                );

                // Set session back to pending (ready for more messages)
                // Sessions work indefinitely - only "failed" is a terminal state
                self.update_session_status("pending", None, None).await?;
            }
            Err(e) => {
                // Handle specific error types
                match &e {
                    everruns_agent_loop::AgentLoopError::NoMessages => {
                        warn!(session_id = %self.session_id, "No messages to process");
                        self.update_session_status("pending", None, None).await?;
                    }
                    everruns_agent_loop::AgentLoopError::MaxIterationsReached(max) => {
                        warn!(session_id = %self.session_id, max = max, "Max iterations reached");
                        // Still set to pending - can continue with more messages
                        self.update_session_status("pending", None, None).await?;
                    }
                    _ => {
                        error!(session_id = %self.session_id, error = %e, "Agent loop failed");
                        // Emit error event and set status to failed
                        let error_event = AgUiEvent::session_error(e.to_string());
                        self.persist_activity
                            .persist_event(self.session_id, error_event)
                            .await?;
                        self.update_session_status("failed", None, Some(Utc::now()))
                            .await?;
                        return Err(e.into());
                    }
                }
            }
        }

        info!(
            session_id = %self.session_id,
            "Session workflow cycle completed, ready for more messages"
        );

        Ok(())
    }

    /// Handle workflow cancellation
    pub async fn cancel(&self) -> Result<()> {
        info!(session_id = %self.session_id, "Cancelling session workflow");

        // Update session status to failed and set finished_at
        self.update_session_status("failed", None, Some(Utc::now()))
            .await?;

        Ok(())
    }

    /// Handle workflow errors
    pub async fn handle_error(&self, error: &anyhow::Error) -> Result<()> {
        error!(
            session_id = %self.session_id,
            error = %error,
            "Session workflow failed"
        );

        // Emit SESSION_ERROR event (SSE notification)
        let error_event = AgUiEvent::session_error(error.to_string());
        self.persist_activity
            .persist_event(self.session_id, error_event)
            .await?;

        // Update session status to failed and set finished_at
        self.update_session_status("failed", None, Some(Utc::now()))
            .await?;

        Ok(())
    }

    /// Update the session timestamps and status
    async fn update_session_status(
        &self,
        status: &str,
        started_at: Option<chrono::DateTime<Utc>>,
        finished_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        let input = UpdateSession {
            status: Some(status.to_string()),
            started_at,
            finished_at,
            ..Default::default()
        };

        self.db.update_session(self.session_id, input).await?;
        Ok(())
    }
}

// Keep the old name as an alias for backwards compatibility during migration
pub type AgentRunWorkflow = SessionWorkflow;

impl AgentRunWorkflow {
    /// Legacy compatibility constructor
    /// In M2, run_id maps to session_id, agent_id remains agent_id
    pub async fn legacy_new(
        run_id: Uuid,
        agent_id: Uuid,
        _thread_id: Uuid,
        db: Database,
    ) -> Result<Self> {
        SessionWorkflow::new(run_id, agent_id, db).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests would require a database connection
    // Unit tests for the workflow structure

    #[test]
    fn test_workflow_type_alias() {
        // Verify the type alias works
        fn _accepts_workflow(_w: AgentRunWorkflow) {}
        fn _accepts_session(_w: SessionWorkflow) {}
    }
}
