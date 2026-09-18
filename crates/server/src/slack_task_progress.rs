//! A live task summary for a Slack thread (EVE-1026).
//!
//! # Rendered, not narrated
//!
//! The tempting shortcut is telling the model to report its task list each
//! turn. That is unreliable — the model forgets, or invents progress — it costs
//! tokens on every turn, and it puts a machine-checkable fact behind a
//! probabilistic step. `task.created` and `task.updated` already carry the real
//! state, so the summary is derived from those and the model is never asked.
//!
//! # One message, updated in place
//!
//! A fan-out of twenty workers settling in quick succession must not become
//! twenty messages, and must not become twenty `chat.update` calls either. This
//! type is an accumulator with a dirty flag: events fold in as they arrive, and
//! the delivery loop's existing 500 ms flush tick — the same cadence the
//! streaming work established (EVE-974), rather than a second rhythm — decides
//! when to push. Update volume is therefore bounded by time, not by fan-out.
//!
//! # Nothing to say, nothing posted
//!
//! An agent that simply answers spawns no tasks, so [`TaskProgress::is_empty`]
//! stays true and no status message is ever created. A thread only gains one
//! when there is fan-out to report.

use std::collections::HashMap;

use everruns_core::session_task::SessionTaskState;

/// Above this many tasks the summary stops naming them and reports counts only.
///
/// Naming is the useful part of a small fan-out ("waiting on `db-migration`")
/// and noise in a large one, where a Slack message would become a wall of
/// worker names nobody reads.
const MAX_NAMED_TASKS: usize = 5;

/// Names longer than this are elided; a display name is model-chosen and
/// otherwise unbounded.
const MAX_NAME_CHARS: usize = 48;

/// What one task contributes to the summary.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskEntry {
    display_name: String,
    state: SessionTaskState,
    /// Creation order, so the rendered list is stable rather than reordering
    /// itself on every update as a hash map would.
    seq: usize,
}

/// Live task state for one turn's fan-out.
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskProgress {
    tasks: HashMap<String, TaskEntry>,
    next_seq: usize,
    /// Whether the rendered text has changed since the last push.
    dirty: bool,
    /// `ts` of the status message, once one has been posted.
    message_ts: Option<String>,
}

impl TaskProgress {
    /// Fold one `task.created` / `task.updated` observation in.
    ///
    /// Takes the three fields the summary renders rather than the whole
    /// `SessionTask`: everything else on a task — spec, artifacts, result path,
    /// heartbeats — changes constantly and would mark the summary dirty for a
    /// message that reads identically.
    ///
    /// Idempotent per state, so a task reporting only a `state_detail` change
    /// costs no Slack call.
    pub fn observe(&mut self, id: &str, display_name: &str, state: SessionTaskState) {
        let display_name = truncate_name(display_name);
        match self.tasks.get_mut(id) {
            Some(entry) => {
                if entry.state != state || entry.display_name != display_name {
                    entry.state = state;
                    entry.display_name = display_name;
                    self.dirty = true;
                }
            }
            None => {
                let seq = self.next_seq;
                self.next_seq += 1;
                self.tasks.insert(
                    id.to_string(),
                    TaskEntry {
                        display_name,
                        state,
                        seq,
                    },
                );
                self.dirty = true;
            }
        }
    }

    /// Whether this turn has any fan-out to report.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Whether the rendered text has changed since the last push.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn message_ts(&self) -> Option<&str> {
        self.message_ts.as_deref()
    }

    /// Record that the current text has been pushed as `ts`.
    pub fn mark_posted(&mut self, ts: String) {
        self.message_ts = Some(ts);
        self.dirty = false;
    }

    /// Record that the current text has been pushed to the existing message.
    pub fn mark_updated(&mut self) {
        self.dirty = false;
    }

    /// The summary line for the thread.
    ///
    /// `final_state` drops the in-flight framing: a turn that has ended must
    /// not leave a message reading "2 running" forever.
    pub fn render(&self, final_state: bool) -> String {
        let count_of = |state: SessionTaskState| {
            self.tasks
                .values()
                .filter(|entry| entry.state == state)
                .count()
        };
        let total = self.tasks.len();
        let done = self
            .tasks
            .values()
            .filter(|entry| entry.state.is_terminal())
            .count();

        let headline = format!("*Tasks* — {done} of {total} finished");

        let mut parts: Vec<String> = Vec::new();
        // Ordered deliberately rather than by map iteration: the reader wants
        // the exceptions first.
        for (state, label) in [
            (SessionTaskState::Failed, "failed"),
            (SessionTaskState::Canceled, "canceled"),
            (SessionTaskState::AwaitingInput, "awaiting input"),
            (SessionTaskState::Running, "running"),
            (SessionTaskState::Queued, "queued"),
        ] {
            let count = count_of(state);
            if count > 0 {
                parts.push(format!("{count} {label}"));
            }
        }

        let mut text = headline;
        if !parts.is_empty() {
            text.push_str(&format!("\n{}", parts.join(" · ")));
        }
        // A turn can end with tasks still unsettled — a background task outlives
        // the turn that spawned it. Saying so is the difference between a
        // summary that ended and one that froze.
        if final_state && done < total {
            text.push_str("\n_Turn ended; remaining tasks continue in the background._");
        }

        if total <= MAX_NAMED_TASKS {
            let mut named: Vec<&TaskEntry> = self.tasks.values().collect();
            named.sort_by_key(|entry| entry.seq);
            for entry in named {
                text.push_str(&format!(
                    "\n{} {}",
                    state_icon(entry.state),
                    entry.display_name
                ));
            }
        }

        text
    }
}

/// A glyph per state, so a reader scans the list rather than reading it.
fn state_icon(state: SessionTaskState) -> &'static str {
    match state {
        SessionTaskState::Queued => "•",
        SessionTaskState::Running => "◐",
        SessionTaskState::AwaitingInput => "?",
        SessionTaskState::Succeeded => "✓",
        SessionTaskState::Failed => "✗",
        SessionTaskState::Canceled => "–",
    }
}

fn truncate_name(name: &str) -> String {
    let name = name.trim();
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_string();
    }
    let kept: String = name.chars().take(MAX_NAME_CHARS - 1).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_with_no_tasks_reports_nothing() {
        let progress = TaskProgress::default();
        assert!(progress.is_empty());
        assert!(!progress.is_dirty());
        assert!(progress.message_ts().is_none());
    }

    #[test]
    fn the_first_task_makes_the_summary_dirty() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "worker-a", SessionTaskState::Queued);
        assert!(!progress.is_empty());
        assert!(progress.is_dirty());
    }

    /// Update volume is the issue's first watch-for item: an update that does
    /// not change the rendered text must not cost a `chat.update`.
    #[test]
    fn an_update_that_changes_nothing_stays_clean() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "worker-a", SessionTaskState::Running);
        progress.mark_posted("1.2".to_string());
        assert!(!progress.is_dirty());

        progress.observe("task_1", "worker-a", SessionTaskState::Running);
        assert!(
            !progress.is_dirty(),
            "a repeated state must not schedule another Slack call"
        );

        progress.observe("task_1", "worker-a", SessionTaskState::Succeeded);
        assert!(progress.is_dirty(), "a real state change must push");
    }

    #[test]
    fn a_settled_task_updates_in_place_rather_than_duplicating() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "worker-a", SessionTaskState::Running);
        progress.mark_posted("1.2".to_string());
        progress.observe("task_1", "worker-a", SessionTaskState::Succeeded);
        progress.mark_updated();

        assert_eq!(progress.message_ts(), Some("1.2"));
        assert!(!progress.is_dirty());
        assert_eq!(
            progress.tasks.len(),
            1,
            "the task was updated, not re-added"
        );
    }

    #[test]
    fn the_headline_counts_finished_against_total() {
        let mut progress = TaskProgress::default();
        for (id, state) in [
            ("task_1", SessionTaskState::Succeeded),
            ("task_2", SessionTaskState::Succeeded),
            ("task_3", SessionTaskState::Succeeded),
            ("task_4", SessionTaskState::Failed),
            ("task_5", SessionTaskState::Running),
        ] {
            progress.observe(id, id, state);
        }
        let text = progress.render(false);
        // Failed counts as finished: it settled. The breakdown below says how.
        assert!(text.contains("4 of 5 finished"), "{text}");
        assert!(text.contains("1 failed"), "{text}");
        assert!(text.contains("1 running"), "{text}");
    }

    /// The exceptions are what a reader is scanning for, so they lead.
    #[test]
    fn the_breakdown_puts_failures_first() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "a", SessionTaskState::Queued);
        progress.observe("task_2", "b", SessionTaskState::Running);
        progress.observe("task_3", "c", SessionTaskState::Failed);

        let text = progress.render(false);
        let failed = text.find("1 failed").expect("failed is reported");
        let running = text.find("1 running").expect("running is reported");
        let queued = text.find("1 queued").expect("queued is reported");
        assert!(failed < running && running < queued, "{text}");
    }

    #[test]
    fn a_small_fan_out_names_its_tasks() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "db-migration", SessionTaskState::Running);
        progress.observe("task_2", "cache-warm", SessionTaskState::Succeeded);

        let text = progress.render(false);
        assert!(text.contains("db-migration"), "{text}");
        assert!(text.contains("cache-warm"), "{text}");
    }

    /// A large fan-out would become a wall of worker names, so it reports
    /// counts only.
    #[test]
    fn a_large_fan_out_reports_counts_only() {
        let mut progress = TaskProgress::default();
        for i in 0..MAX_NAMED_TASKS + 1 {
            progress.observe(
                &format!("task_{i}"),
                &format!("worker-{i}"),
                SessionTaskState::Running,
            );
        }
        let text = progress.render(false);
        assert!(!text.contains("worker-0"), "{text}");
        assert!(text.contains("0 of 6 finished"), "{text}");
        assert!(text.contains("6 running"), "{text}");
    }

    /// The rendered order must not shuffle between updates, which a hash map's
    /// iteration order would do.
    #[test]
    fn named_tasks_keep_their_creation_order() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "first", SessionTaskState::Running);
        progress.observe("task_2", "second", SessionTaskState::Running);
        progress.observe("task_3", "third", SessionTaskState::Running);

        for _ in 0..5 {
            let text = progress.render(false);
            let first = text.find("first").expect("first");
            let second = text.find("second").expect("second");
            let third = text.find("third").expect("third");
            assert!(first < second && second < third, "{text}");
        }
    }

    /// "the summary reflects the final state rather than freezing mid-flight":
    /// a turn that ends with tasks still running must say so.
    #[test]
    fn a_final_render_says_the_turn_ended_with_work_outstanding() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "a", SessionTaskState::Succeeded);
        progress.observe("task_2", "b", SessionTaskState::Running);

        assert!(!progress.render(false).contains("Turn ended"));
        assert!(
            progress.render(true).contains("Turn ended"),
            "{}",
            progress.render(true)
        );
    }

    /// ...and one that ends with everything settled must not.
    #[test]
    fn a_final_render_with_everything_settled_adds_no_note() {
        let mut progress = TaskProgress::default();
        progress.observe("task_1", "a", SessionTaskState::Succeeded);
        progress.observe("task_2", "b", SessionTaskState::Failed);

        let text = progress.render(true);
        assert!(!text.contains("Turn ended"), "{text}");
        assert!(text.contains("2 of 2 finished"), "{text}");
    }

    #[test]
    fn a_model_chosen_name_cannot_flood_the_message() {
        let mut progress = TaskProgress::default();
        let long = "n".repeat(MAX_NAME_CHARS * 4);
        progress.observe("task_1", &long, SessionTaskState::Running);
        let text = progress.render(false);
        assert!(
            !text.contains(&long),
            "an unbounded display name must be elided"
        );
        assert!(text.contains('…'), "{text}");
    }
}
