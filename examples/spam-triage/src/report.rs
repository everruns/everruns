//! What the run is graded on, and how it prints.
//!
//! Kept apart from the pipeline so the numbers are computed once, from the
//! records, rather than accumulated while printing.

use std::fmt;

use crate::pipeline::Triage;

/// The counts a run is judged by.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Tally {
    pub total: usize,
    pub escalated: usize,
    /// Correct if the screen had decided every email on its own.
    pub screen_only_correct: usize,
    /// Correct after the escalations were applied.
    pub pipeline_correct: usize,
    /// Escalations where the larger model changed the answer to the right one.
    pub rescued: usize,
    /// Escalations where it changed a right answer to a wrong one.
    pub broken: usize,
    /// Escalations whose answer named neither verdict.
    pub unreadable: usize,
}

impl Tally {
    pub fn of(triages: &[Triage]) -> Self {
        let mut tally = Tally {
            total: triages.len(),
            ..Tally::default()
        };
        for triage in triages {
            let screen_correct = triage.screening.verdict() == triage.email.label;
            let pipeline_correct = triage.correct();
            tally.screen_only_correct += usize::from(screen_correct);
            tally.pipeline_correct += usize::from(pipeline_correct);
            if let Some(adjudication) = &triage.adjudication {
                tally.escalated += 1;
                tally.unreadable += usize::from(adjudication.verdict.is_none());
                tally.rescued += usize::from(!screen_correct && pipeline_correct);
                tally.broken += usize::from(screen_correct && !pipeline_correct);
            }
        }
        tally
    }

    /// Share of the corpus the screen settled by itself.
    pub fn settled(&self) -> usize {
        self.total - self.escalated
    }

    pub fn screen_only_accuracy(&self) -> f64 {
        percent(self.screen_only_correct, self.total)
    }

    pub fn pipeline_accuracy(&self) -> f64 {
        percent(self.pipeline_correct, self.total)
    }
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 * 100.0 / whole as f64
}

/// How one email's outcome reads in the per-message list.
pub struct Row<'a>(pub &'a Triage);

impl fmt::Display for Row<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let triage = self.0;
        // A wrong final verdict is the only thing worth scanning the list for.
        let mark = if triage.correct() { ' ' } else { '!' };
        let route = if triage.escalated() {
            "routed"
        } else {
            "screened"
        };
        write!(
            f,
            "{mark} {:<16} {:<10} {:>7.2}  {:<10} {:<8} {}",
            triage.email.id,
            triage.email.label.to_string(),
            triage.screening.spam_probability,
            triage.verdict().to_string(),
            route,
            triage.email.short_subject(44),
        )
    }
}

/// The header for [`Row`], aligned with it.
pub const ROW_HEADER: &str = "  MESSAGE          LABEL      P(SPAM)  VERDICT    ROUTE    SUBJECT";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{Email, Label};
    use crate::pipeline::{Adjudication, Screening};

    fn triage(label: Label, spam_probability: f64, escalated: Option<Option<Label>>) -> Triage {
        Triage {
            email: Email {
                id: "1".into(),
                sender: "a@b.test".into(),
                subject: "Subject".into(),
                body: "Body".into(),
                label,
            },
            screening: Screening {
                spam_probability,
                input_tokens: 4,
            },
            adjudication: escalated.map(|verdict| Adjudication {
                verdict,
                total_tokens: Some(9),
                cost_usd: Some(0.001),
            }),
        }
    }

    #[test]
    fn the_tally_separates_what_the_screen_got_from_what_routing_added() {
        let tally = Tally::of(&[
            // Settled and right.
            triage(Label::Spam, 0.99, None),
            // Escalated; the model flipped a wrong screen to right.
            triage(Label::Legitimate, 0.60, Some(Some(Label::Legitimate))),
            // Escalated; the model broke a right screen.
            triage(Label::Spam, 0.55, Some(Some(Label::Legitimate))),
            // Escalated; unreadable, so the screen's wrong answer stands.
            triage(Label::Spam, 0.40, Some(None)),
        ]);
        assert_eq!(tally.total, 4);
        assert_eq!(tally.escalated, 3);
        assert_eq!(tally.settled(), 1);
        assert_eq!(tally.screen_only_correct, 2);
        assert_eq!(tally.pipeline_correct, 2);
        assert_eq!(tally.rescued, 1);
        assert_eq!(tally.broken, 1);
        assert_eq!(tally.unreadable, 1);
        assert!((tally.pipeline_accuracy() - 50.0).abs() < 1e-9);
        assert!((tally.screen_only_accuracy() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn an_empty_run_reports_zero_rather_than_dividing_by_zero() {
        let tally = Tally::of(&[]);
        assert_eq!(tally.pipeline_accuracy(), 0.0);
        assert_eq!(tally.screen_only_accuracy(), 0.0);
    }

    #[test]
    fn a_wrong_row_is_marked_and_names_its_route() {
        let rendered = Row(&triage(Label::Spam, 0.55, Some(Some(Label::Legitimate)))).to_string();
        assert!(rendered.starts_with('!'), "{rendered}");
        assert!(rendered.contains("routed"), "{rendered}");
        let screened = Row(&triage(Label::Spam, 0.99, None)).to_string();
        assert!(screened.starts_with(' '), "{screened}");
        assert!(screened.contains("screened"), "{screened}");
    }

    #[test]
    fn the_header_lines_up_with_the_rows_under_it() {
        // Distinct label and verdict, so each lookup can only match its own column.
        let rendered = Row(&triage(Label::Legitimate, 0.55, Some(Some(Label::Spam)))).to_string();
        for (header, cell) in [
            ("MESSAGE", "1"),
            ("LABEL", "legitimate"),
            ("VERDICT", "spam"),
            ("SUBJECT", "Subject"),
        ] {
            assert_eq!(
                ROW_HEADER.find(header),
                rendered.find(cell),
                "{header} column is misaligned:\n{ROW_HEADER}\n{rendered}"
            );
        }
    }
}
