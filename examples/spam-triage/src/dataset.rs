//! The labeled corpus the pipeline reads, and nothing else.
//!
//! The messages are not vendored: see `data/build-corpus.sh`, which downloads
//! the SpamAssassin public corpus and writes the JSONL this module loads.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

/// Spam or not. The corpus supplies it; the pipeline is graded against it and
/// never sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Label {
    Spam,
    Legitimate,
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Label::Spam => "spam",
            Label::Legitimate => "legitimate",
        })
    }
}

/// One message, already decoded and truncated by the corpus builder.
#[derive(Debug, Clone, Deserialize)]
pub struct Email {
    pub id: String,
    pub sender: String,
    pub subject: String,
    pub body: String,
    pub label: Label,
}

impl Email {
    /// What both stages are shown. Identical text for the classifier and the
    /// model, so the comparison between them is about the judgment and not
    /// about who got more to read.
    pub fn as_state(&self) -> String {
        format!(
            "From: {}\nSubject: {}\n\n{}",
            self.sender, self.subject, self.body
        )
    }

    /// The subject, bounded for a terminal column.
    pub fn short_subject(&self, width: usize) -> String {
        let subject = if self.subject.is_empty() {
            "(no subject)"
        } else {
            self.subject.as_str()
        };
        let mut short: String = subject.chars().take(width).collect();
        if subject.chars().count() > width {
            short.push('…');
        }
        short
    }
}

/// Read a JSONL corpus, failing on the first malformed line rather than
/// silently triaging a shorter set than the report claims.
pub fn load(path: &Path) -> io::Result<Vec<Email>> {
    let contents = fs::read_to_string(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{}: {error}\nBuild it first: bash examples/spam-triage/data/build-corpus.sh",
                path.display()
            ),
        )
    })?;
    parse(&contents)
}

fn parse(contents: &str) -> io::Result<Vec<Email>> {
    let mut emails = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let email: Email = serde_json::from_str(line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("line {}: {error}", index + 1),
            )
        })?;
        emails.push(email);
    }
    if emails.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "corpus is empty",
        ));
    }
    Ok(emails)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str =
        r#"{"id":"1","sender":"a@b.test","subject":"Hi","body":"Text","label":"spam"}"#;

    #[test]
    fn a_corpus_line_parses_into_a_labeled_email() {
        let emails = parse(LINE).unwrap();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].label, Label::Spam);
        assert_eq!(emails[0].as_state(), "From: a@b.test\nSubject: Hi\n\nText");
    }

    #[test]
    fn blank_lines_are_skipped_but_a_malformed_one_is_an_error() {
        assert_eq!(parse(&format!("\n{LINE}\n\n")).unwrap().len(), 1);
        let error = parse(&format!("{LINE}\n{{\"id\":\"2\"}}")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().starts_with("line 2:"), "{error}");
    }

    #[test]
    fn an_empty_corpus_is_an_error_not_a_perfect_score() {
        assert!(parse("   \n\n").is_err());
    }

    #[test]
    fn a_long_subject_is_truncated_with_a_marker() {
        let email = &parse(
            r#"{"id":"1","sender":"a","subject":"abcdefghij","body":"b","label":"legitimate"}"#,
        )
        .unwrap()[0];
        assert_eq!(email.short_subject(4), "abcd…");
        assert_eq!(email.short_subject(10), "abcdefghij");
    }

    #[test]
    fn a_missing_subject_still_prints_something() {
        let email =
            &parse(r#"{"id":"1","sender":"a","subject":"","body":"b","label":"spam"}"#).unwrap()[0];
        assert_eq!(email.short_subject(40), "(no subject)");
    }
}
