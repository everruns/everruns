// Message conversion for the chat-completions protocol.
//
// Split out of `openai_protocol.rs`, which is at its size cap, when tool-result
// images needed handling (EVE-1133).

use crate::message::{LlmContentPart, Message, MessageContent, MessageRole};
use crate::openai_types::{
    OpenAiContent, OpenAiContentPart, OpenAiFile, OpenAiFunctionCall, OpenAiImageUrl,
    OpenAiInputAudio, OpenAiMessage, OpenAiToolCall,
};

/// Convert one message for the wire, splitting a tool result that carries
/// images into the `tool` message plus a following `user` message.
///
/// Chat-completions takes a `tool` message's `content` as a string. An
/// image part on one is not rejected — it is dropped or ignored — so a
/// screenshot answering a computer-use call never reaches the model and
/// the agent reasons about a tool result it cannot see (EVE-1133). `user`
/// is the role this API accepts image parts on, so the images follow the
/// result there, labelled with the call they answer so the pairing
/// survives.
///
/// The Responses protocol has a real `computer_call_output` for this and
/// does not need the workaround.
fn convert_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

pub(crate) fn convert_message(msg: &Message) -> OpenAiMessage {
    let content = match &msg.content {
        MessageContent::Text(text) => OpenAiContent::Text(text.clone()),
        MessageContent::Parts(parts) => {
            let openai_parts: Vec<OpenAiContentPart> = parts
                .iter()
                .filter_map(|part| match part {
                    LlmContentPart::Text { text } => Some(OpenAiContentPart::Text {
                        r#type: "text".to_string(),
                        text: text.clone(),
                    }),
                    LlmContentPart::Image { url } => Some(OpenAiContentPart::ImageUrl {
                        r#type: "image_url".to_string(),
                        image_url: OpenAiImageUrl { url: url.clone() },
                    }),
                    LlmContentPart::Audio { url } => Some(OpenAiContentPart::InputAudio {
                        r#type: "input_audio".to_string(),
                        input_audio: OpenAiInputAudio {
                            data: url.clone(),
                            format: "wav".to_string(),
                        },
                    }),
                    LlmContentPart::File { url, filename } => Some(OpenAiContentPart::File {
                        r#type: "file".to_string(),
                        file: OpenAiFile {
                            filename: filename.clone(),
                            file_data: url.clone(),
                        },
                    }),
                    LlmContentPart::ProviderOpaque(_) => None,
                })
                .collect();
            OpenAiContent::Parts(openai_parts)
        }
    };

    // OpenAI only accepts tool_calls on assistant messages
    let tool_calls = if msg.role == MessageRole::Assistant {
        msg.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .map(|tc| OpenAiToolCall {
                    id: tc.id.clone(),
                    r#type: "function".to_string(),
                    function: OpenAiFunctionCall {
                        name: tc.name.clone(),
                        arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    },
                })
                .collect()
        })
    } else {
        None
    };

    OpenAiMessage {
        role: convert_role(&msg.role).to_string(),
        content: Some(content),
        tool_calls,
        tool_call_id: msg.tool_call_id.clone(),
    }
}

pub(crate) fn convert_message_seq(msg: &Message) -> Vec<OpenAiMessage> {
    let converted = convert_message(msg);
    if msg.role != MessageRole::Tool {
        return vec![converted];
    }
    let MessageContent::Parts(parts) = &msg.content else {
        return vec![converted];
    };
    let images: Vec<OpenAiContentPart> = parts
        .iter()
        .filter_map(|part| match part {
            LlmContentPart::Image { url } => Some(OpenAiContentPart::ImageUrl {
                r#type: "image_url".to_string(),
                image_url: OpenAiImageUrl { url: url.clone() },
            }),
            _ => None,
        })
        .collect();
    if images.is_empty() {
        return vec![converted];
    }

    // The tool message keeps the text. Joining rather than dropping the
    // non-image parts keeps a result that mixed prose and a screenshot
    // intact; an image-only result leaves an empty string, which the API
    // accepts and which reads correctly next to the image that follows.
    let text = parts
        .iter()
        .filter_map(|part| match part {
            LlmContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut tool_message = converted;
    tool_message.content = Some(OpenAiContent::Text(text));

    let label = match msg.tool_call_id.as_deref() {
        Some(id) => format!("Image output from tool call {id}."),
        None => "Image output from the preceding tool call.".to_string(),
    };
    let mut carrier = Vec::with_capacity(images.len() + 1);
    carrier.push(OpenAiContentPart::Text {
        r#type: "text".to_string(),
        text: label,
    });
    carrier.extend(images);

    vec![
        tool_message,
        OpenAiMessage {
            role: "user".to_string(),
            content: Some(OpenAiContent::Parts(carrier)),
            tool_calls: None,
            tool_call_id: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// EVE-1133: chat-completions takes a `tool` message's content as a string.
    /// An image part on one is not an error the provider reports — it is
    /// dropped or ignored, so a screenshot answering a computer-use call never
    /// reaches the model and the agent reasons about a tool result it cannot
    /// see. Carry the images on a following `user` message, which is the one
    /// role this API accepts image parts on.
    #[test]
    fn tool_result_images_move_to_a_user_message() {
        use crate::message::{LlmContentPart, MessageContent};
        let mut tool = Message::text(MessageRole::Tool, "");
        tool.tool_call_id = Some("call".into());
        tool.content = MessageContent::Parts(vec![
            LlmContentPart::Text {
                text: "clicked Submit".into(),
            },
            LlmContentPart::Image {
                url: "data:image/png;base64,AAAA".into(),
            },
        ]);

        let wire: Vec<_> = [tool].iter().flat_map(convert_message_seq).collect();

        assert_eq!(
            serde_json::to_value(wire).unwrap(),
            json!([
                {"role":"tool","content":"clicked Submit","tool_call_id":"call"},
                {"role":"user","content":[
                    {"type":"text","text":"Image output from tool call call."},
                    {"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}
                ]}
            ])
        );
    }

    /// A tool result with no image is untouched: one message, string content,
    /// no stray user turn inserted into the transcript.
    #[test]
    fn text_only_tool_results_are_unchanged() {
        let mut tool = Message::text(MessageRole::Tool, "file content");
        tool.tool_call_id = Some("call".into());
        let wire: Vec<_> = [tool].iter().flat_map(convert_message_seq).collect();
        assert_eq!(
            serde_json::to_value(wire).unwrap(),
            json!([{"role":"tool","content":"file content","tool_call_id":"call"}])
        );
    }

    /// Images on a user message are already legal there and must not be moved,
    /// duplicated, or relabelled.
    #[test]
    fn user_message_images_are_left_alone() {
        use crate::message::{LlmContentPart, MessageContent};
        let mut user = Message::text(MessageRole::User, "");
        user.content = MessageContent::Parts(vec![LlmContentPart::Image {
            url: "data:image/png;base64,BBBB".into(),
        }]);
        let wire: Vec<_> = [user].iter().flat_map(convert_message_seq).collect();
        assert_eq!(
            serde_json::to_value(wire).unwrap(),
            json!([{"role":"user","content":[
                {"type":"image_url","image_url":{"url":"data:image/png;base64,BBBB"}}
            ]}])
        );
    }
}
