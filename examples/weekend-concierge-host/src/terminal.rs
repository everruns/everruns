//! A terminal [`AskUser`] responder: numbered options, toggles, free text, and
//! an unechoed prompt for a credential.
//!
//! Kept local to this example rather than moved into `examples/demo-support`.
//! This crate is deliberately excluded from the repository workspace so it
//! demonstrates a standalone external consumer of `everruns`; depending on an
//! in-workspace helper would undo that. If a second example ever needs a
//! terminal responder, that is the moment to lift this into `demo-support`.

use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::sync::{Arc, Mutex};

use everruns::ask_user::{
    Answer, AnsweredBy, AskUser, Outcome, Question, QuestionKind, Status, async_trait,
    session_secret_ref,
};

/// Where a collected credential goes.
///
/// Stands in for the encrypted session-secret store a hosted deployment has.
/// The point is that it is the *host's*: the value is written here and the
/// answer carries only a `secret_ref`, so nothing the model sees — and nothing
/// persisted as a tool result — ever holds the credential itself.
#[derive(Clone, Default)]
pub struct HostSecrets(Arc<Mutex<HashMap<String, String>>>);

impl HostSecrets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a credential was stored under `name`. Deliberately not a getter:
    /// an example has no business printing one back.
    pub fn holds(&self, name: &str) -> bool {
        self.0
            .lock()
            .map(|map| map.contains_key(name))
            .unwrap_or(false)
    }

    fn store(&self, name: &str, value: String) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(name.to_string(), value);
        }
    }
}

/// Answers `ask_user` questions by talking to whoever is at the terminal.
pub struct TerminalResponder {
    secrets: HostSecrets,
}

impl TerminalResponder {
    pub fn new(secrets: HostSecrets) -> Self {
        Self { secrets }
    }
}

/// Read one line, or `None` at end of input.
fn read_line() -> Option<String> {
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line).ok()?;
    (read > 0).then(|| line.trim().to_string())
}

/// Read a line without echoing it.
///
/// Uses `stty` rather than a crate so this example stays dependency-free. If
/// the terminal will not turn echo off, say so rather than silently printing
/// the credential as it is typed — a surprise here is the whole problem.
fn read_secret_line() -> Option<String> {
    let quiet = std::process::Command::new("stty")
        .arg("-echo")
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !quiet {
        println!("  (warning: could not disable echo; what you type will be visible)");
    }
    let line = read_line();
    if quiet {
        let _ = std::process::Command::new("stty").arg("echo").status();
        println!();
    }
    line
}

/// The labels a question declares as default, which is what an unattended or
/// skipped answer falls back to.
fn declared_defaults(question: &Question) -> Vec<String> {
    let defaults: Vec<String> = question
        .options
        .iter()
        .filter(|option| option.is_default)
        .map(|option| option.label.clone())
        .collect();
    if defaults.is_empty() {
        question
            .options
            .first()
            .map(|option| option.label.clone())
            .into_iter()
            .collect()
    } else {
        defaults
    }
}

fn print_options(question: &Question) {
    for (index, option) in question.options.iter().enumerate() {
        let marker = if option.is_default { " (default)" } else { "" };
        println!(
            "  {}. {} — {}{marker}",
            index + 1,
            option.label,
            option.description
        );
    }
    if question.allow_other {
        println!("  {}. Other…", question.options.len() + 1);
    }
}

/// Map a typed choice number onto a label, or `None` for the "Other" slot.
fn pick(question: &Question, token: &str) -> Option<Option<String>> {
    let index: usize = token.trim().parse().ok()?;
    if index >= 1 && index <= question.options.len() {
        Some(Some(question.options[index - 1].label.clone()))
    } else if question.allow_other && index == question.options.len() + 1 {
        Some(None)
    } else {
        None
    }
}

impl TerminalResponder {
    fn ask_choice(&self, question: &Question) -> Answer {
        println!("\n{} — {}", question.header, question.question);
        print_options(question);
        if question.multi_select {
            println!("Choose any (comma-separated numbers), or Enter for the defaults:");
        } else {
            println!("Choose one number, or Enter for the default:");
        }
        print!("> ");
        let _ = std::io::stdout().flush();

        let typed = read_line().unwrap_or_default();
        if typed.is_empty() {
            return Answer {
                id: question.id.clone().unwrap_or_default(),
                selected: declared_defaults(question),
                other_text: None,
                secret_ref: None,
            };
        }

        let mut selected = Vec::new();
        let mut other_text = None;
        // A multi-select reads every token; a single-select honours the first
        // and ignores the rest rather than silently widening the answer.
        let tokens: Vec<&str> = typed.split(',').collect();
        for token in tokens {
            match pick(question, token) {
                Some(Some(label)) => selected.push(label),
                Some(None) => {
                    print!("  Other (free text): ");
                    let _ = std::io::stdout().flush();
                    other_text = read_line().filter(|text| !text.is_empty());
                }
                None => println!("  (ignoring {:?}: not one of the choices)", token.trim()),
            }
            if !question.multi_select && (!selected.is_empty() || other_text.is_some()) {
                break;
            }
        }
        if selected.is_empty() && other_text.is_none() {
            selected = declared_defaults(question);
        }

        Answer {
            id: question.id.clone().unwrap_or_default(),
            selected,
            other_text,
            secret_ref: None,
        }
    }

    fn ask_secret(&self, question: &Question) -> Answer {
        let name = question.secret_name.clone().unwrap_or_default();
        println!("\n{} — {}", question.header, question.question);
        if let Some(purpose) = &question.purpose {
            println!("  Used for: {purpose}");
        }
        println!("  Stored as {name}; the agent receives a reference, never the value.");
        print!("> ");
        let _ = std::io::stdout().flush();

        let typed = read_secret_line().unwrap_or_default();
        if typed.is_empty() {
            // Nothing collected means nothing to reference. Leaving `secret_ref`
            // absent is what tells the model to proceed without the credential
            // rather than assume one is waiting for it.
            return Answer {
                id: question.id.clone().unwrap_or_default(),
                selected: Vec::new(),
                other_text: None,
                secret_ref: None,
            };
        }
        self.secrets.store(&name, typed);
        Answer {
            id: question.id.clone().unwrap_or_default(),
            selected: Vec::new(),
            other_text: None,
            secret_ref: Some(session_secret_ref(&name)),
        }
    }
}

#[async_trait]
impl AskUser for TerminalResponder {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        // Nobody is at the keyboard under `cargo test`, in CI, or behind a pipe.
        // Falling back to the declared defaults keeps those runs from blocking
        // forever on a prompt no one will ever see.
        if !std::io::stdin().is_terminal() {
            return Outcome {
                status: Status::Answered,
                answered_by: AnsweredBy::Unattended,
                answers: questions
                    .iter()
                    .map(|question| Answer {
                        id: question.id.clone().unwrap_or_default(),
                        selected: match question.kind {
                            QuestionKind::Secret => Vec::new(),
                            QuestionKind::Choice => declared_defaults(question),
                        },
                        other_text: None,
                        secret_ref: None,
                    })
                    .collect(),
            };
        }

        let answers = questions
            .iter()
            .map(|question| match question.kind {
                QuestionKind::Secret => self.ask_secret(question),
                QuestionKind::Choice => self.ask_choice(question),
            })
            .collect();
        Outcome {
            status: Status::Answered,
            answered_by: AnsweredBy::User,
            answers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::ask_user::AskUserOption;

    fn choice(multi: bool) -> Question {
        Question {
            kind: QuestionKind::Choice,
            id: Some("vibe".into()),
            header: "Vibe".into(),
            question: "Which vibe?".into(),
            multi_select: multi,
            allow_other: true,
            options: vec![
                AskUserOption {
                    label: "playful".into(),
                    description: "Games".into(),
                    is_default: true,
                },
                AskUserOption {
                    label: "cozy".into(),
                    description: "Quiet".into(),
                    is_default: false,
                },
            ],
            secret_name: None,
            purpose: None,
        }
    }

    #[test]
    fn a_question_with_no_declared_default_falls_back_to_its_first_option() {
        let mut question = choice(false);
        question.options[0].is_default = false;
        assert_eq!(declared_defaults(&question), vec!["playful".to_string()]);
    }

    #[test]
    fn declared_defaults_can_be_several_on_a_multi_select() {
        let mut question = choice(true);
        question.options[1].is_default = true;
        assert_eq!(
            declared_defaults(&question),
            vec!["playful".to_string(), "cozy".to_string()]
        );
    }

    #[test]
    fn numbers_map_onto_labels_and_the_other_slot() {
        let question = choice(false);
        assert_eq!(pick(&question, "1"), Some(Some("playful".to_string())));
        assert_eq!(pick(&question, " 2 "), Some(Some("cozy".to_string())));
        // One past the options is "Other…" when the question allows it.
        assert_eq!(pick(&question, "3"), Some(None));
        assert_eq!(pick(&question, "4"), None);
        assert_eq!(pick(&question, "0"), None);
        assert_eq!(pick(&question, "nope"), None);
    }

    #[test]
    fn a_question_that_forbids_other_has_no_other_slot() {
        let mut question = choice(false);
        question.allow_other = false;
        assert_eq!(pick(&question, "3"), None);
    }

    #[tokio::test]
    async fn without_a_terminal_it_answers_unattended_from_the_defaults() {
        // This is the path `cargo test` and CI take, and the reason neither
        // blocks on a prompt.
        let responder = TerminalResponder::new(HostSecrets::new());
        let outcome = responder.ask(&[choice(false)]).await;
        assert_eq!(outcome.answered_by, AnsweredBy::Unattended);
        assert_eq!(outcome.answers[0].selected, vec!["playful".to_string()]);
        assert_eq!(outcome.answers[0].secret_ref, None);
    }

    #[test]
    fn a_stored_secret_is_reachable_by_name_and_never_by_value() {
        let secrets = HostSecrets::new();
        assert!(!secrets.holds("WEATHER_API_KEY"));
        secrets.store("WEATHER_API_KEY", "sk-not-a-real-key".into());
        assert!(secrets.holds("WEATHER_API_KEY"));
        // The handle the model would receive names the secret, nothing more.
        assert_eq!(
            session_secret_ref("WEATHER_API_KEY"),
            "session:WEATHER_API_KEY"
        );
    }
}
