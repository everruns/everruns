//! Stateless endpoints require a valid completion or an explicit terminal error.
use crate::{LlmStreamError, LlmStreamEvent};
use serde_json::Value;

/// An explicitly incomplete resource can never certify a complete summary,
/// even when a malformed reason claims `stop`. Keep the raw reason in metadata.
pub(super) fn incomplete_reason(reason: Option<&str>) -> String {
    match crate::CompletionOutcome::from_finish_reason(reason) {
        crate::CompletionOutcome::OutputLimit => "length",
        crate::CompletionOutcome::Refused => "content_filter",
        crate::CompletionOutcome::Cancelled => "cancelled",
        crate::CompletionOutcome::Failed => "error",
        _ => "incomplete",
    }
    .to_owned()
}

pub(super) fn terminal(frame: &Value) -> Option<LlmStreamEvent> {
    match frame["type"].as_str()? {
        "response.completed"
            if frame["response"]["status"] == "completed"
                && frame["response"]["output"].is_array() =>
        {
            None
        }
        "response.failed" | "error" => {
            let error = frame
                .pointer("/response/error")
                .or_else(|| frame.get("error"))
                .unwrap_or(frame);
            Some(LlmStreamEvent::Error(LlmStreamError::provider(
                Some(
                    error["code"]
                        .as_str()
                        .unwrap_or("processing_error")
                        .to_owned(),
                ),
                None,
                error["message"]
                    .as_str()
                    .unwrap_or("The provider failed while processing the response."),
            )))
        }
        "response.completed" | "response.incomplete" | "response.done" => {
            Some(LlmStreamEvent::Error(LlmStreamError::provider(
                Some("malformed_response".to_owned()),
                None,
                "The provider did not return a valid completed response.",
            )))
        }
        _ => None,
    }
}
