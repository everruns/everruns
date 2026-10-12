//! Content of an Anthropic `tool_result` block.
//!
//! Plain text when the tool returned only text; otherwise an array of text,
//! image and document blocks. Documents carry PDFs a tool returned, such as
//! an attachment `read_file` reopened after compaction.

use everruns_contracts::driver_helpers::parse_data_url;
use everruns_contracts::driver_registry::{LlmContentPart, MessageContent};

use super::{
    AnthropicDocumentSource, AnthropicImageSource, AnthropicToolResultBlock,
    AnthropicToolResultContent,
};

pub(super) fn tool_result_content(content: &MessageContent) -> AnthropicToolResultContent {
    let MessageContent::Parts(parts) = content else {
        return AnthropicToolResultContent::Text(content.to_text());
    };
    let has_media = parts.iter().any(|part| {
        matches!(
            part,
            LlmContentPart::Image { .. } | LlmContentPart::File { .. }
        )
    });
    if !has_media {
        return AnthropicToolResultContent::Text(content.to_text());
    }
    let blocks = parts
        .iter()
        .filter_map(|part| match part {
            LlmContentPart::Text { text } => {
                Some(AnthropicToolResultBlock::Text { text: text.clone() })
            }
            LlmContentPart::Image { url } => {
                let source = match parse_data_url(url) {
                    Some(parsed) => AnthropicImageSource::Base64 {
                        media_type: parsed.media_type,
                        data: parsed.data,
                    },
                    None => AnthropicImageSource::Url { url: url.clone() },
                };
                Some(AnthropicToolResultBlock::Image { source })
            }
            LlmContentPart::File { url, .. } => {
                let source = match parse_data_url(url) {
                    Some(parsed) => AnthropicDocumentSource::Base64 {
                        media_type: parsed.media_type,
                        data: parsed.data,
                    },
                    None => AnthropicDocumentSource::Url { url: url.clone() },
                };
                Some(AnthropicToolResultBlock::Document { source })
            }
            _ => None,
        })
        .collect();
    AnthropicToolResultContent::Blocks(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pdf_in_tool_result_becomes_a_document_block() {
        let content = MessageContent::Parts(vec![
            LlmContentPart::text("{\"path\":\"/attachments/a1/report.pdf\"}"),
            LlmContentPart::file("data:application/pdf;base64,JVBERi0=", None),
        ]);

        let value = serde_json::to_value(tool_result_content(&content)).unwrap();

        assert_eq!(
            value,
            json!([
                {"type": "text", "text": "{\"path\":\"/attachments/a1/report.pdf\"}"},
                {"type": "document", "source": {
                    "type": "base64", "media_type": "application/pdf", "data": "JVBERi0="
                }}
            ])
        );
    }

    #[test]
    fn text_only_tool_result_stays_a_string() {
        let content = MessageContent::Parts(vec![LlmContentPart::text("done")]);

        let value = serde_json::to_value(tool_result_content(&content)).unwrap();

        assert_eq!(value, json!("done"));
    }
}
