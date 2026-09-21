use super::AgentBuilder;

impl AgentBuilder {
    /// Enable `ask_user` with an in-process host responder.
    ///
    /// The responder runs inside the tool call, so the turn does not park for
    /// a browser or server. Calling `.capability("ask_user")` without this
    /// method uses [`ask_user::DefaultsResponder`](crate::ask_user::DefaultsResponder)
    /// and resolves unattended choices immediately.
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::ask_user::{AskUser, Outcome, Question, async_trait};
    /// use everruns::{Agent, Model};
    ///
    /// struct Terminal;
    ///
    /// #[async_trait]
    /// impl AskUser for Terminal {
    ///     async fn ask(&self, questions: &[Question]) -> Outcome {
    ///         # let _ = questions;
    ///         # unimplemented!("read answers from the terminal")
    ///     }
    /// }
    ///
    /// let agent = Agent::builder()
    ///     .instructions("Ask before choosing a deployment target.")
    ///     .model(Model::simulated("Done."))
    ///     .ask_user(Terminal)
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok::<(), everruns::BuildError>(())
    /// ```
    pub fn ask_user(mut self, responder: impl crate::ask_user::AskUser + 'static) -> Self {
        self.capabilities
            .push(crate::CapabilityRef::new(everruns_builtins::ASK_USER_CAPABILITY_ID).into());
        self.ask_user = Some(everruns_builtins::AskUserCapability::new(responder));
        self
    }
}
