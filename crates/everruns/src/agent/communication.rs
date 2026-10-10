//! How an agent talks: the builder side of [`Communication`].

use super::AgentBuilder;
use crate::conversation::Communication;

impl AgentBuilder {
    /// How the agent talks to people. The default,
    /// [`Communication::Direct`](Communication::Direct),
    /// makes its text the reply. With
    /// [`Communication::Explicit`](Communication::Explicit)
    /// its text becomes private working notes and it talks only through the
    /// `send_message` tool (or decides not to answer with `no_reply`), so a
    /// host shows people only what the agent chose to send.
    ///
    /// ```
    /// use everruns::conversation::Communication;
    ///
    /// let builder = everruns::Agent::builder()
    ///     .name("support")
    ///     .instructions("Help customers with their orders.")
    ///     .communication(Communication::Explicit);
    /// # let _ = builder;
    /// ```
    pub fn communication(mut self, communication: Communication) -> Self {
        self.communication = communication;
        self
    }
}
