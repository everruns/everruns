//! Terminal presentation shared by the Framework example agents.
//!
//! Agent behavior — tools, instructions, provider, capabilities — lives in each
//! example's `main.rs`. This crate only prints, so nothing here changes what an
//! agent does: replace [`run`] with [`Session::send_and_wait`] and the behavior
//! is identical, minus the live view of each tool call.
//!
//! [`run`] suits agents whose tools return prose; [`shell::run`] suits agents
//! driving a shell, where the payload is an exec result worth decoding.
//!
//! These observers print tool arguments and results. The example agents expose
//! public or self-contained demo data; review what a tool can return before
//! pointing an observer at one that handles private data.

pub mod shell;

use everruns::{Session, SessionEventKind, Turn};
use serde_json::Value;

/// Send `question` and print each tool call with a bounded preview of its result.
pub async fn run(session: &Session, question: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    show("QUESTION", question);
    let mut events = session.events();
    let pending = session.send(question).await?;
    while let Some(event) = events.recv().await? {
        if event.turn_id.as_deref() != Some(pending.turn_id.as_str()) {
            continue;
        }
        let terminal = event.kind.is_terminal();
        match &event.kind {
            SessionEventKind::ToolStarted { tool_name, .. } => {
                println!("\n> {tool_name}");
                let data = event.canonical_json();
                if let Some(arguments) = data["data"]["tool_call"]["arguments"].as_object() {
                    for (key, value) in arguments {
                        let value = value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string());
                        text(&format!("  {key}: {value}"));
                    }
                }
            }
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => {
                // These example tools return public/demo data. Do not dump arbitrary
                // canonical event payloads from tools that handle private data.
                let data = event.canonical_json();
                let result = &data["data"]["result"];
                let result_text = result
                    .as_array()
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_else(|| {
                        data["data"]["error"]
                            .as_str()
                            .unwrap_or("No text result")
                            .to_string()
                    });
                println!(
                    "\n< {tool_name}: {}",
                    if *success { "OK" } else { "FAILED" }
                );
                text(&preview(&result_text));
            }
            _ => {}
        }
        if terminal {
            break;
        }
    }
    let turn = pending.wait().await?;
    if !turn.success {
        return Err(turn
            .error
            .unwrap_or_else(|| format!("Turn ended: {:?}", turn.stop_reason))
            .into());
    }
    println!("\nANSWER\n");
    text(&turn.response);
    println!(
        "\nCompleted: {} iterations, {} tool calls",
        turn.iterations, turn.tool_calls
    );
    Ok(turn)
}

/// Print a labeled block without changing indentation or code layout.
pub fn show(label: &str, text: &str) {
    println!("\n{label}");
    self::text(text);
}

fn preview(body: &str) -> String {
    // Only public/demo data belongs here; review this before using private tools.
    let parsed = serde_json::from_str::<Value>(body).ok();
    if let Some(value) = &parsed
        && let Some(results) = value["results"].as_array()
    {
        return results
            .iter()
            .take(3)
            .map(|result| {
                format!(
                    "  {}\n  {}",
                    result["title"].as_str().unwrap_or("Source"),
                    result["url"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(value) = &parsed
        && value["content"].as_str().is_none()
        && let Some(fields) = value.as_object()
    {
        return fields
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    if parsed.is_none() && body.starts_with("{\"content\":") {
        return "Structured source content received; preview omitted because the result was truncated."
            .into();
    }
    let body = parsed
        .as_ref()
        .and_then(|value| value["content"].as_str())
        .unwrap_or(body);
    let excerpt: String = body.chars().take(650).collect();
    if body.chars().count() > 650 {
        format!("{excerpt}\n[excerpt; full result supplied to agent]")
    } else {
        excerpt
    }
}

fn text(body: &str) {
    let safe: String = body
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect();
    for line in safe.lines() {
        println!("{}", line.trim_end());
    }
}

#[cfg(test)]
mod tests {
    use super::preview;

    #[test]
    fn fetch_preview_displays_content_instead_of_transport_json() {
        assert_eq!(
            preview(r#"{"content":"Source text\nSecond line","status":200}"#),
            "Source text\nSecond line"
        );
    }

    #[test]
    fn search_preview_keeps_source_identity() {
        let result = preview(
            r#"{"results":[{"title":"Primary source","url":"https://example.org","description":"long body"}]}"#,
        );
        assert!(result.contains("Primary source"));
        assert!(result.contains("https://example.org"));
        assert!(!result.contains("description"));
    }
}
