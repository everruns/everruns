//! Colored presentation for agents that drive a shell.
//!
//! Renders each script the agent runs and decodes the exec payload it gets back
//! (exit status plus bounded stdout and stderr) instead of printing the raw JSON
//! envelope. Colors are ANSI escapes written unconditionally unless `NO_COLOR`
//! is set, so a piped transcript (see an example's `record.sh`) keeps them for
//! replay under `less -R`.

use std::sync::OnceLock;

use everruns::{Session, SessionEventKind, Turn};

/// No styling; the default for body text.
pub const PLAIN: &str = "";

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
/// Dim style, exported for callers that print quoted file content.
pub const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

/// Longest line the demo prints, chosen to fit the recorded terminal.
const WIDTH: usize = 104;
/// Lines kept from one shell script.
const MAX_SCRIPT_LINES: usize = 12;
/// Lines kept from one command's stdout or stderr.
const MAX_OUTPUT_LINES: usize = 6;

/// `true` unless `NO_COLOR` is set; read once.
fn colored() -> bool {
    static COLORED: OnceLock<bool> = OnceLock::new();
    *COLORED.get_or_init(|| std::env::var_os("NO_COLOR").is_none())
}

/// An escape sequence, or nothing when color is disabled.
fn sgr(code: &str) -> &str {
    if colored() { code } else { "" }
}

fn paint(code: &str, text: &str) -> String {
    if code.is_empty() {
        return text.to_string();
    }
    format!("{}{text}{}", sgr(code), sgr(RESET))
}

/// Clip one line to the recorded terminal width.
fn clip(line: &str) -> String {
    if line.chars().count() <= WIDTH {
        return line.to_string();
    }
    let kept: String = line.chars().take(WIDTH - 1).collect();
    format!("{kept}…")
}

/// Title bar printed once at startup.
pub fn banner(title: &str) {
    println!("{}", paint(&format!("{BOLD}{CYAN}"), title));
    println!("{}", paint(DIM, &"─".repeat(WIDTH)));
}

/// A `label  value` line under the banner.
pub fn field(label: &str, value: &str) {
    println!("{} {}", paint(DIM, &format!("{label:>12}")), clip(value));
}

/// A section heading with a blank line above it.
pub fn section(label: &str) {
    println!("\n{}", paint(&format!("{BOLD}{MAGENTA}"), label));
}

/// Indented body text in one style.
pub fn body(text: &str, code: &str) {
    for line in text.lines() {
        println!("  {}", paint(code, &clip(line)));
    }
}

/// Body text, capped at `limit` lines with a dim note about the remainder.
fn body_capped(text: &str, code: &str, prefix: &str, limit: usize) {
    let lines: Vec<&str> = text.lines().collect();
    for line in lines.iter().take(limit) {
        println!("  {}{}", prefix, paint(code, &clip(line)));
    }
    if lines.len() > limit {
        let rest = lines.len() - limit;
        println!("  {}", paint(DIM, &format!("… {rest} more line(s)")));
    }
}

/// One post-run verification line.
pub fn check(passed: bool, label: &str) {
    let mark = if passed {
        paint(GREEN, "✓")
    } else {
        paint(&format!("{BOLD}{RED}"), "✗")
    };
    let text = if passed {
        label.to_string()
    } else {
        paint(RED, label)
    };
    println!("  {mark} {text}");
}

/// Render one `bash` tool result: exit status plus bounded output.
///
/// The Bashkit capability returns a JSON payload (`exit_code`, `stdout`,
/// `stderr`, `success`); showing the fields beats dumping the envelope.
fn shell_result(text: &str) {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
        body_capped(text, DIM, "", MAX_OUTPUT_LINES);
        return;
    };
    let exit_code = payload["exit_code"].as_i64().unwrap_or(-1);
    let status = if exit_code == 0 {
        paint(GREEN, "exit 0")
    } else {
        paint(RED, &format!("exit {exit_code}"))
    };
    println!("  {status}");
    for (stream, code) in [("stdout", DIM), ("stderr", YELLOW)] {
        let content = payload[stream].as_str().unwrap_or_default();
        if !content.trim().is_empty() {
            body_capped(
                content.trim_end(),
                code,
                &paint(DIM, "│ "),
                MAX_OUTPUT_LINES,
            );
        }
    }
}

/// Send `request` and print every shell command the agent runs while it works.
pub async fn run(session: &Session, request: &str) -> Result<Turn, Box<dyn std::error::Error>> {
    section("REQUEST");
    body(request, PLAIN);
    let mut events = session.events();
    let pending = session.send(request).await?;
    while let Some(event) = events.recv().await? {
        if event.turn_id.as_deref() != Some(pending.turn_id.as_str()) {
            continue;
        }
        let terminal = event.kind.is_terminal();
        match &event.kind {
            SessionEventKind::ReasonStarted => {
                println!("\n{}", paint(DIM, "· calling the model"));
            }
            SessionEventKind::ToolStarted { tool_name, .. } => {
                let arguments = &event.canonical_json()["data"]["tool_call"]["arguments"];
                let script = arguments["commands"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| arguments.to_string());
                section(&format!("❯ {tool_name}"));
                body_capped(&script, BLUE, "", MAX_SCRIPT_LINES);
            }
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => {
                // Review what a tool can return before printing its payload
                // verbatim over a workspace that holds private data.
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
                if *success {
                    shell_result(&text);
                } else {
                    println!("  {}", paint(RED, &format!("{tool_name} failed")));
                    body_capped(&text, RED, "", MAX_OUTPUT_LINES);
                }
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
    section("SUMMARY");
    body(&turn.response, PLAIN);
    println!(
        "\n{}",
        paint(
            DIM,
            &format!(
                "{} iterations · {} tool calls",
                turn.iterations, turn.tool_calls
            )
        )
    );
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::{PLAIN, WIDTH, clip, paint};

    #[test]
    fn clip_keeps_short_lines_and_truncates_long_ones_on_char_boundaries() {
        assert_eq!(clip("short"), "short");
        // A line of multi-byte characters must not be cut mid-character.
        let wide = "é".repeat(WIDTH + 10);
        let clipped = clip(&wide);
        assert_eq!(clipped.chars().count(), WIDTH);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn plain_style_adds_no_escapes() {
        assert_eq!(paint(PLAIN, "text"), "text");
    }
}
