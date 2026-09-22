//! Answering an agent's questions from a real terminal.
//!
//! This is the half of `ask_user` an embedding host owns. The contract types
//! come from `everruns::ask_user`; how a person supplies an answer is entirely
//! the host's business — a TUI, a Slack message, a desktop dialog. Here it is
//! stdin.
//!
//! Deliberately not in `examples/demo-support`: that crate promises its
//! observers never change what an agent does, and a responder decides what the
//! agent is told, which is the opposite. It moves there only if it stops being
//! the thing that answers.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::Mutex;

use everruns::ask_user::{
    Answer, AnsweredBy, AskUser, Outcome, Question, QuestionKind, Status, async_trait,
};

/// Reads answers from stdin: numbered options, a free-text path when the
/// question allows one, and echo off for a credential.
pub struct TerminalResponder {
    /// Credentials this host collected, by name.
    ///
    /// The value stays here. The answer the model reads carries only
    /// `secret_ref`, so a credential never reaches the transcript, the event
    /// log, or model context — the whole point of the secret kind.
    secrets: Mutex<HashMap<String, String>>,
}

impl Default for TerminalResponder {
    fn default() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }
}

impl TerminalResponder {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the host collected, for a tool to resolve a `secret_ref` against.
    pub fn secret(&self, name: &str) -> Option<String> {
        self.secrets.lock().ok()?.get(name).cloned()
    }

    fn prompt(&self, question: &Question) -> Answer {
        let id = question.id.clone().unwrap_or_default();
        println!("\n[{}] {}", question.header, question.question);

        if question.kind == QuestionKind::Secret {
            return self.prompt_secret(question, id);
        }

        for (index, option) in question.options.iter().enumerate() {
            let marker = if option.is_default { "*" } else { " " };
            println!(
                "  {}{}. {} — {}",
                marker,
                index + 1,
                option.label,
                option.description
            );
        }
        if question.allow_other {
            println!("   {}. Something else", question.options.len() + 1);
        }
        if question.multi_select {
            println!("  (several, comma-separated)");
        }

        let raw = read_line("> ");
        self.resolve_choice(question, id, &raw)
    }

    fn resolve_choice(&self, question: &Question, id: String, raw: &str) -> Answer {
        let other_index = question.options.len() + 1;
        let mut selected = Vec::new();
        let mut other_text = None;

        for token in raw.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            match token.parse::<usize>() {
                Ok(n) if n == other_index && question.allow_other => {
                    other_text = Some(read_line("  what instead? "));
                }
                Ok(n) if (1..=question.options.len()).contains(&n) => {
                    selected.push(question.options[n - 1].label.clone());
                }
                // Anything unparseable falls through to free text when the
                // question allows it, so a person who typed a name rather than
                // a number is not silently ignored.
                _ if question.allow_other => other_text = Some(token.to_string()),
                _ => {}
            }
            if !question.multi_select && (!selected.is_empty() || other_text.is_some()) {
                break;
            }
        }

        // An empty answer takes the declared default rather than sending the
        // model nothing to act on.
        if selected.is_empty() && other_text.is_none() {
            selected.extend(
                question
                    .options
                    .iter()
                    .find(|option| option.is_default)
                    .or_else(|| question.options.first())
                    .map(|option| option.label.clone()),
            );
        }

        Answer {
            id,
            selected,
            other_text: other_text.filter(|text| !text.trim().is_empty()),
            secret_ref: None,
        }
    }

    fn prompt_secret(&self, question: &Question, id: String) -> Answer {
        let name = question.secret_name.clone().unwrap_or_default();
        if let Some(purpose) = &question.purpose {
            println!("  used for: {purpose}");
        }
        let value = read_secret_line(&format!("  {name}: "));
        self.store_secret(&name, value, id)
    }

    /// Keep the value here and answer with a reference.
    ///
    /// Split from the prompt so the shape of the answer can be tested without
    /// a terminal — the invariant worth pinning is that no value leaves.
    fn store_secret(&self, name: &str, value: String, id: String) -> Answer {
        if let Ok(mut secrets) = self.secrets.lock() {
            secrets.insert(name.to_string(), value);
        }
        Answer {
            id,
            selected: Vec::new(),
            other_text: None,
            secret_ref: Some(everruns::ask_user::session_secret_ref(name)),
        }
    }
}

#[async_trait]
impl AskUser for TerminalResponder {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        let answers = questions.iter().map(|q| self.prompt(q)).collect();
        Outcome {
            status: Status::Answered,
            // A person actually typed these.
            answered_by: AnsweredBy::User,
            answers,
        }
    }
}

fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_string()
}

/// Read one line without echoing it, so a credential never lands on the screen
/// or in the terminal's scrollback.
///
/// Restores the terminal's original settings even if reading fails; a shell
/// left with echo off is worse than a failed prompt.
fn read_secret_line(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();

    let restore = unsafe { disable_echo() };
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    if let Some(original) = restore {
        unsafe { restore_termios(&original) };
    }
    println!();
    line.trim().to_string()
}

/// Turn off terminal echo, returning the previous settings to restore.
///
/// Returns `None` when stdin is not a terminal — a pipe or a test harness —
/// in which case there is no echo to disable and nothing to restore.
unsafe fn disable_echo() -> Option<libc::termios> {
    unsafe {
        if libc::isatty(libc::STDIN_FILENO) != 1 {
            return None;
        }
        let mut current: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut current) != 0 {
            return None;
        }
        let original = current;
        current.c_lflag &= !libc::ECHO;
        if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &current) != 0 {
            return None;
        }
        Some(original)
    }
}

unsafe fn restore_termios(original: &libc::termios) {
    unsafe {
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, original);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn question(value: serde_json::Value) -> Question {
        serde_json::from_value(value).expect("fixture matches the contract")
    }

    fn choice(multi_select: bool, allow_other: bool) -> Question {
        question(json!({
            "kind": "choice", "id": "energy", "header": "Energy",
            "question": "How much energy?",
            "multi_select": multi_select, "allow_other": allow_other,
            "options": [
                {"label": "Up for anything", "description": "Loud.", "default": true},
                {"label": "Low-key", "description": "Quiet."},
                {"label": "Undecided", "description": "Ask later."}
            ]
        }))
    }

    #[test]
    fn a_number_selects_that_option() {
        let answer = TerminalResponder::new().resolve_choice(&choice(false, true), "e".into(), "2");
        assert_eq!(answer.selected, vec!["Low-key".to_string()]);
        assert!(answer.other_text.is_none());
    }

    #[test]
    fn a_multi_select_takes_several() {
        let answer =
            TerminalResponder::new().resolve_choice(&choice(true, true), "e".into(), "1, 3");
        assert_eq!(
            answer.selected,
            vec!["Up for anything".to_string(), "Undecided".to_string()]
        );
    }

    #[test]
    fn a_single_select_stops_at_the_first() {
        let answer =
            TerminalResponder::new().resolve_choice(&choice(false, true), "e".into(), "2, 3");
        assert_eq!(answer.selected, vec!["Low-key".to_string()]);
    }

    /// Someone who typed a name rather than a number should not be ignored.
    #[test]
    fn unparseable_input_becomes_free_text_when_allowed() {
        let answer = TerminalResponder::new().resolve_choice(
            &choice(false, true),
            "e".into(),
            "somewhere with a pool",
        );
        assert_eq!(answer.other_text.as_deref(), Some("somewhere with a pool"));

        // ...but a closed question falls back to the default instead of
        // inventing an option that was never offered.
        let closed =
            TerminalResponder::new().resolve_choice(&choice(false, false), "e".into(), "nonsense");
        assert_eq!(closed.selected, vec!["Up for anything".to_string()]);
        assert!(closed.other_text.is_none());
    }

    #[test]
    fn an_empty_answer_takes_the_declared_default() {
        let answer = TerminalResponder::new().resolve_choice(&choice(false, true), "e".into(), "");
        assert_eq!(answer.selected, vec!["Up for anything".to_string()]);
    }

    /// The point of the secret kind: the value stays with the host and the
    /// answer carries only a reference.
    #[test]
    fn a_secret_answer_carries_no_value() {
        let responder = TerminalResponder::new();
        let answer = responder.store_secret("OPENTABLE_TOKEN", "hunter2".into(), "tok".into());

        assert_eq!(
            answer.secret_ref.as_deref(),
            Some("session:OPENTABLE_TOKEN")
        );
        assert!(answer.selected.is_empty());
        assert!(answer.other_text.is_none());

        let rendered = serde_json::to_string(&answer).expect("serialises");
        assert!(!rendered.contains("hunter2"), "{rendered}");

        // The host kept it, so a tool can still resolve the reference.
        assert_eq!(
            responder.secret("OPENTABLE_TOKEN").as_deref(),
            Some("hunter2")
        );
    }
}
