//! Bounded textual projection, including actual tool outcomes.
use super::runner::{DreamEvidence, MAX_EVIDENCE_BYTES};
use crate::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};
use std::io::{self, Write};

struct BoundedText {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedText {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("dreaming evidence byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn text(message: &RuntimeMessage, limit: usize) -> Result<Option<String>, ()> {
    let mut output = BoundedText {
        bytes: Vec::new(),
        limit,
    };
    for part in &message.content {
        match part {
            ContentPart::Text(part) => output.write_all(part.text.as_bytes()).map_err(|_| ())?,
            ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => {
                // Preserve call IDs, arguments, results AND errors: an
                // assistant claim alone is not proof a tool succeeded.
                serde_json::to_writer(&mut output, part).map_err(|_| ())?;
            }
            // Exclude images, attachments and reasoning from textual evidence.
            _ => continue,
        }
        output.write_all(b"\n").map_err(|_| ())?;
    }
    if output.bytes.is_empty() {
        return Ok(None);
    }
    String::from_utf8(output.bytes).map(Some).map_err(|_| ())
}

pub(crate) fn select(messages: &[RuntimeMessage]) -> Vec<DreamEvidence> {
    let mut evidence = Vec::new();
    let mut bytes = 0;
    for message in messages.iter().rev().take(256) {
        let role = match message.role {
            RuntimeMessageRole::User => "user",
            RuntimeMessageRole::Agent => "assistant",
            RuntimeMessageRole::ToolResult => "tool",
            _ => continue,
        };
        let selected = match text(message, MAX_EVIDENCE_BYTES - bytes) {
            Ok(Some(text)) => text,
            Ok(None) => continue,
            Err(()) => break,
        };
        bytes += selected.len();
        evidence.push(DreamEvidence {
            id: message.id.to_string(),
            role: role.into(),
            text: selected,
        });
    }
    evidence.reverse();
    evidence
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_requests_results_and_errors_remain_source_linked() {
        let mut request = RuntimeMessage::assistant("");
        request.content = vec![ContentPart::tool_call(
            "check",
            "run_tests",
            json!({"isolated_database":true}),
        )];
        let result = RuntimeMessage::tool_result("check", Some(json!({"passed":42})), None);
        let failed =
            RuntimeMessage::tool_result("retry", None, Some("database unavailable".into()));
        let evidence = select(&[request, result.clone(), failed.clone()]);
        assert_eq!(evidence.len(), 3);
        assert!(evidence[0].text.contains("run_tests"));
        assert_eq!(evidence[1].id, result.id.to_string());
        assert_eq!(evidence[1].role, "tool");
        assert!(evidence[1].text.contains("passed"));
        assert_eq!(evidence[2].id, failed.id.to_string());
        assert!(evidence[2].text.contains("database unavailable"));
    }

    #[test]
    fn all_text_parts_are_kept_and_image_payloads_are_excluded() {
        let mut message = RuntimeMessage::user("Old region.");
        message
            .content
            .push(ContentPart::text("Correction: eu-west-1."));
        message.content.push(ContentPart::image_url(
            "data:image/png;base64,private-image",
        ));
        let evidence = select(&[message]);
        assert!(evidence[0].text.contains("Old region."));
        assert!(evidence[0].text.contains("eu-west-1"));
        assert!(!evidence[0].text.contains("private-image"));
    }

    #[test]
    fn oversized_results_cannot_expand_the_selected_window() {
        let huge = RuntimeMessage::tool_result(
            "huge",
            Some(json!({"body":"x".repeat(MAX_EVIDENCE_BYTES*2)})),
            None,
        );
        let latest = RuntimeMessage::assistant("The result needs review.");
        let evidence = select(&[huge, latest.clone()]);
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].id, latest.id.to_string());
        assert!(evidence.iter().map(|e| e.text.len()).sum::<usize>() <= MAX_EVIDENCE_BYTES);
    }
}
