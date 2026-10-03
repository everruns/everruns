//! Answering a run's interrupts on the run that resumes it.
//!
//! The 1.0 coverage rule: a resuming run answers **every** interrupt the
//! previous run left open, each exactly once, either resolving it or
//! abandoning it (`cancelled`), and answers nothing else. The consumer
//! enforces it before sending, so a producer never has to guess what an
//! omitted interrupt meant.

use std::collections::HashSet;

use serde_json::Value;

use crate::ag_ui::{Interrupt, Metadata, ResumeEntry, ResumeStatus};

/// A resume list that breaks the coverage rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeError {
    /// The id names no open interrupt.
    UnknownInterrupt(String),
    /// The interrupt was already answered.
    DuplicateAnswer(String),
    /// These open interrupts have no answer.
    Uncovered(Vec<String>),
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownInterrupt(id) => write!(f, "no open interrupt has id '{id}'"),
            Self::DuplicateAnswer(id) => write!(f, "interrupt '{id}' is answered twice"),
            Self::Uncovered(ids) => write!(
                f,
                "a resume must answer every open interrupt; unanswered: {}",
                ids.join(", ")
            ),
        }
    }
}

impl std::error::Error for ResumeError {}

/// Builds the `resume` list for a run that continues an interrupted one.
///
/// ```
/// use everruns_core::ag_ui::{Interrupt, ResumeBuilder, ResumeError, ResumeStatus};
/// use serde_json::json;
///
/// let open = vec![
///     Interrupt::new("approve-1", "tool_approval"),
///     Interrupt::new("ask-1", "input_required"),
/// ];
///
/// // Leaving an interrupt unanswered is refused.
/// let mut partial = ResumeBuilder::new(&open);
/// partial.resolve("ask-1", json!({ "answer": "blue" })).unwrap();
/// assert_eq!(
///     partial.build(),
///     Err(ResumeError::Uncovered(vec!["approve-1".into()])),
/// );
///
/// // Answer what you can and abandon the rest explicitly.
/// let mut resume = ResumeBuilder::new(&open);
/// resume.resolve("ask-1", json!({ "answer": "blue" })).unwrap();
/// resume.cancel_remaining();
/// let entries = resume.build().unwrap();
/// assert_eq!(entries[0].interrupt_id, "approve-1");
/// assert_eq!(entries[0].status, ResumeStatus::Cancelled);
/// assert_eq!(entries[1].status, ResumeStatus::Resolved);
/// ```
#[derive(Clone, Debug)]
pub struct ResumeBuilder {
    open: Vec<String>,
    entries: Vec<ResumeEntry>,
}

impl ResumeBuilder {
    /// Starts a resume for the interrupts the previous run ended with.
    pub fn new(interrupts: &[Interrupt]) -> Self {
        Self {
            open: interrupts.iter().map(|i| i.id.clone()).collect(),
            entries: Vec::new(),
        }
    }

    fn check(&self, id: &str) -> Result<(), ResumeError> {
        if !self.open.iter().any(|open| open == id) {
            return Err(ResumeError::UnknownInterrupt(id.to_owned()));
        }
        if self.entries.iter().any(|e| e.interrupt_id == id) {
            return Err(ResumeError::DuplicateAnswer(id.to_owned()));
        }
        Ok(())
    }

    /// Answers an interrupt with `payload`.
    pub fn resolve(&mut self, id: &str, payload: Value) -> Result<&mut Self, ResumeError> {
        self.answer(id, ResumeStatus::Resolved, Some(payload), None)
    }

    /// Abandons an interrupt.
    pub fn cancel(&mut self, id: &str) -> Result<&mut Self, ResumeError> {
        self.answer(id, ResumeStatus::Cancelled, None, None)
    }

    /// Adds a fully specified answer.
    pub fn answer(
        &mut self,
        id: &str,
        status: ResumeStatus,
        payload: Option<Value>,
        metadata: Option<Metadata>,
    ) -> Result<&mut Self, ResumeError> {
        self.check(id)?;
        self.entries.push(ResumeEntry {
            interrupt_id: id.to_owned(),
            status,
            payload,
            metadata,
        });
        Ok(self)
    }

    /// Abandons every interrupt not answered yet.
    pub fn cancel_remaining(&mut self) -> &mut Self {
        let answered: HashSet<String> = self
            .entries
            .iter()
            .map(|e| e.interrupt_id.clone())
            .collect();
        for id in &self.open {
            if !answered.contains(id) {
                self.entries.push(ResumeEntry {
                    interrupt_id: id.clone(),
                    status: ResumeStatus::Cancelled,
                    payload: None,
                    metadata: None,
                });
            }
        }
        self
    }

    /// The resume list, in the order the interrupts were raised. Refuses one
    /// that leaves an interrupt unanswered.
    pub fn build(self) -> Result<Vec<ResumeEntry>, ResumeError> {
        let mut entries = self.entries;
        let uncovered: Vec<String> = self
            .open
            .iter()
            .filter(|id| !entries.iter().any(|e| &e.interrupt_id == *id))
            .cloned()
            .collect();
        if !uncovered.is_empty() {
            return Err(ResumeError::Uncovered(uncovered));
        }
        let order = |id: &str| self.open.iter().position(|open| open == id);
        entries.sort_by_key(|e| order(&e.interrupt_id));
        Ok(entries)
    }
}

/// Checks an already-built resume list against the open interrupts.
///
/// ```
/// use everruns_core::ag_ui::{Interrupt, ResumeEntry, ResumeStatus, check_resume_coverage};
///
/// let open = [Interrupt::new("i1", "approval")];
/// assert!(check_resume_coverage(&open, &[]).is_err());
/// let answered = [ResumeEntry {
///     interrupt_id: "i1".into(),
///     status: ResumeStatus::Cancelled,
///     payload: None,
///     metadata: None,
/// }];
/// assert!(check_resume_coverage(&open, &answered).is_ok());
/// ```
pub fn check_resume_coverage(
    interrupts: &[Interrupt],
    resume: &[ResumeEntry],
) -> Result<(), ResumeError> {
    let mut builder = ResumeBuilder::new(interrupts);
    for entry in resume {
        builder.answer(
            &entry.interrupt_id,
            entry.status,
            entry.payload.clone(),
            entry.metadata.clone(),
        )?;
    }
    builder.build().map(|_| ())
}
