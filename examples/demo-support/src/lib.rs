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
            SessionEventKind::ReasonStarted => println!("\n[Calling model]"),
            SessionEventKind::ToolStarted { tool_name, .. } => {
                let args = &event.canonical_json()["data"]["tool_call"]["arguments"];
                show(&format!("TOOL: {tool_name}"), &args.to_string());
            }
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => {
                // These example tools return public/demo data. Do not dump arbitrary
                // canonical event payloads from tools that handle private data.
                let data = event.canonical_json();
                let result = &data["data"]["result"];
                let text = result
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
                let preview: String = text.chars().take(240).collect();
                let suffix = if text.chars().count() > 240 {
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
    show("ANSWER", &turn.response);
    println!(
        "\nCompleted: {} iterations, {} tool calls",
        turn.iterations, turn.tool_calls
    );
    Ok(turn)
}

/// Print a labeled block, wrapped to 90 columns.
pub fn show(label: &str, text: &str) {
    println!("\n{label}");
    for paragraph in text.lines() {
        let mut column = 0;
        for word in paragraph.split_whitespace() {
            let word: String = word.chars().filter(|c| !c.is_control()).collect();
            if column > 0 && column + word.chars().count() + 1 > 90 {
                println!();
                column = 0;
            }
            if column > 0 {
                print!(" ");
                column += 1;
            }
            for ch in word.chars() {
                if column == 90 {
                    println!();
                    column = 0;
                }
                print!("{ch}");
                column += 1;
            }
        }
        println!();
    }
}
