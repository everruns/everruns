use super::RuntimeMessage;

const TURN_SCOPED_REMINDER_KEY: &str = "everruns.turn_scoped_reminder";

impl RuntimeMessage {
    /// Create a prompt-only system reminder scoped to one model turn.
    pub fn turn_scoped_system(content: impl Into<String>) -> Self {
        let mut message = Self::system(content);
        message.metadata = Some(std::collections::HashMap::from([(
            TURN_SCOPED_REMINDER_KEY.to_string(),
            serde_json::Value::Bool(true),
        )]));
        message
    }

    /// Whether this prompt-facing message expires after the current turn.
    pub fn is_turn_scoped_reminder(&self) -> bool {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.get(TURN_SCOPED_REMINDER_KEY))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
}
