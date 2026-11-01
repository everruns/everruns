//! Mistral connection-level chat API selection.

/// Chat API used by a Mistral provider connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MistralApi {
    /// Chat Completions (the compatibility default).
    #[default]
    ChatCompletions,
    /// Stateless Responses preview.
    Responses,
}

impl MistralApi {
    /// Stable name accepted by hosted provider configuration.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
        }
    }
}
