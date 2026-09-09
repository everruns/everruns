//! Terminal presentation for this example; agent behavior lives in main.rs.
use everruns::{Session, SessionEventKind, Turn};

/// Send `request` and print every shell command the agent runs while it works.
pub async fn run(session: &Session, request: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    show("REQUEST", request);
    let mut events = session.events();
    let pending = session.send(request).await?;
    while let Some(event) = events.recv().await? {
        if event.turn_id.as_deref() != Some(pending.turn_id.as_str()) {
            continue;
        }
        let terminal = event.kind.is_terminal();
        match &event.kind {
            SessionEventKind::ReasonStarted => println!("\n[Calling model]"),
            SessionEventKind::ToolStarted { tool_name, .. } => {
                let arguments = &event.canonical_json()["data"]["tool_call"]["arguments"];
                let script = arguments["commands"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| arguments.to_string());
                show(&format!("$ {tool_name}"), &script);
            }
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => {
                // The shell runs against this example's own throwaway working
                // copy. Review what a tool can return before printing its
                // payload verbatim over a workspace that holds private data.
                let data = event.canonical_json();
                let text = data["data"]["result"]
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
                let preview: String = text.chars().take(400).collect();
                let suffix = if text.chars().count() > 400 {
                    " … [preview]"
                } else {
                    ""
                };
                show(
                    &format!("{tool_name}: {}", if *success { "OK" } else { "FAILED" }),
                    &format!("{preview}{suffix}"),
                );
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
    show("SUMMARY", &turn.response);
    println!(
        "\nCompleted: {} iterations, {} tool calls",
        turn.iterations, turn.tool_calls
    );
    Ok(turn)
}

pub fn show(label: &str, text: &str) {
    println!("\n{label}");
    for line in text.lines() {
        println!("  {line}");
    }
}
