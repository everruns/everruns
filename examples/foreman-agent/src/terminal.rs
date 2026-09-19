//! How a factory run looks while it happens. Nothing here changes a decision.
//!
//! Two streams share one screen on purpose: the worker's own output, dimmed,
//! and the supervisor's readings cutting in while it is still talking. That
//! interleaving is the architecture, so it is what the demo shows.
//!
//! The ANSI layer is [`everruns_example_demo::style`]; only the layout — the
//! nine-dimension block and the decision line — belongs to this example.

use std::io::Write;
use std::sync::Mutex;

use everruns_example_demo::style::{
    BOLD, CYAN, DIM, GREEN, MAGENTA, RED, WIDTH, YELLOW, clip_to, paint,
};

use crate::factory::Watcher;
use crate::foreman::{Assessment, DIMENSIONS, Lens};
use crate::observation::{TestRun, WorkerKind, WorkerRecord, WorkerStatus};
use crate::policy::{Action, Intervention};

/// Lines shown from one shell script.
const SCRIPT_LINES: usize = 3;
/// Width of one probability bar, in cells.
const BAR: usize = 14;
/// Column the dimension labels are padded to.
const LABEL: usize = 24;
/// Characters of worker text on one line, inside the two-space indent.
const WRAP: usize = WIDTH - 4;

/// Clip one line to the indented body width.
fn clip(line: &str) -> String {
    clip_to(line, WRAP)
}

/// Renders a factory run as it happens.
pub struct Terminal {
    /// Worker text not yet printed, so a reading never lands halfway through
    /// one of its lines and no line runs off the recorded terminal.
    pending: Mutex<String>,
}

impl Terminal {
    /// A terminal renderer at the start of a line.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(String::new()),
        }
    }

    /// Flush whatever the worker was mid-sentence on.
    fn break_line(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if !pending.is_empty() {
            println!("  {}", paint(DIM, pending.trim_end()));
            pending.clear();
        }
    }
}

/// Take the next line to print: up to a newline, or a word break before the
/// terminal runs out of room. `None` means keep buffering.
fn next_line(buffer: &mut String) -> Option<String> {
    if let Some(index) = buffer.find('\n') {
        let line = buffer[..index].to_owned();
        buffer.drain(..index + 1);
        return Some(line);
    }
    if buffer.chars().count() <= WRAP {
        return None;
    }
    // A model streams text in chunks of no fixed length, so the break has to be
    // chosen from the buffer rather than tracked as a column count.
    let limit = buffer
        .char_indices()
        .nth(WRAP)
        .map_or(buffer.len(), |(index, _)| index);
    let cut = buffer[..limit].rfind(' ').map_or(limit, |index| index);
    let line = buffer[..cut].to_owned();
    buffer.drain(..cut);
    let trimmed = buffer.trim_start().to_owned();
    *buffer = trimmed;
    Some(line)
}

impl Watcher for Terminal {
    fn worker_started(&self, worker: &WorkerRecord) {
        self.break_line();
        let kind = match worker.kind {
            WorkerKind::Coding => "coding worker",
            WorkerKind::Verifier => "independent verifier",
        };
        println!(
            "\n{} {} {}",
            paint(&format!("{BOLD}{CYAN}"), "▸"),
            paint(BOLD, &worker.id),
            paint(DIM, &format!("{kind}, attempt {}", worker.attempt)),
        );
    }

    fn worker_text(&self, _worker_id: &str, delta: &str) {
        // The worker's own words, dimmed: present, but never competing with the
        // supervisor's numbers.
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.push_str(delta);
        while let Some(line) = next_line(&mut pending) {
            println!("  {}", paint(DIM, line.trim_end()));
        }
        let _ = std::io::stdout().flush();
    }

    fn worker_tool(&self, _worker_id: &str, tool: &str, script: &str) {
        self.break_line();
        println!("  {} {}", paint(DIM, "❯"), paint(YELLOW, tool));
        let lines: Vec<&str> = script
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        for line in lines.iter().take(SCRIPT_LINES) {
            println!("    {}", paint(DIM, &clip(line)));
        }
        if lines.len() > SCRIPT_LINES {
            println!(
                "    {}",
                paint(
                    DIM,
                    &format!("… {} more line(s)", lines.len() - SCRIPT_LINES)
                )
            );
        }
    }

    fn worker_finished(&self, worker: &WorkerRecord) {
        self.break_line();
        let (code, label) = match worker.status {
            WorkerStatus::Completed => (GREEN, "completed"),
            WorkerStatus::Stopped => (YELLOW, "stopped"),
            WorkerStatus::Failed => (RED, "failed"),
            WorkerStatus::Running => (DIM, "running"),
        };
        println!(
            "  {} {} {}",
            paint(DIM, "└"),
            paint(code, label),
            paint(
                DIM,
                &format!(
                    "after {:.0}s, {} tool call(s)",
                    worker.elapsed().as_secs_f64(),
                    worker.tool_calls
                )
            ),
        );
    }

    fn assessed(&self, iteration: usize, assessment: &Assessment) {
        self.break_line();
        println!(
            "\n  {} {}",
            paint(
                &format!("{BOLD}{MAGENTA}"),
                &format!("foreman · reading {iteration}")
            ),
            paint(DIM, &"─".repeat(WIDTH.saturating_sub(22))),
        );
        for lens in [Lens::Job, Lens::Floor] {
            let heading = match lens {
                Lens::Job => "the job",
                Lens::Floor => "the floor",
            };
            println!("  {}", paint(DIM, heading));
            for dimension in DIMENSIONS.iter().filter(|d| d.lens == lens) {
                let value = assessment.value(dimension.id);
                println!(
                    "    {:LABEL$} {} {}",
                    dimension.id,
                    bar(value),
                    paint(BOLD, &format!("{value:.2}")),
                );
            }
        }
    }

    fn assessment_failed(&self, iteration: usize, error: &str) {
        self.break_line();
        println!(
            "\n  {} {}",
            paint(
                &format!("{BOLD}{RED}"),
                &format!("foreman · reading {iteration} failed")
            ),
            paint(DIM, error),
        );
    }

    fn tested(&self, run: &TestRun) {
        self.break_line();
        let (code, label) = if run.passed {
            (GREEN, "tests passed")
        } else {
            (RED, "tests failed")
        };
        let headline = run
            .output_tail
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default();
        println!(
            "  {} {} {}",
            paint(DIM, "·"),
            paint(code, label),
            paint(DIM, &clip(headline)),
        );
    }

    fn intervened(&self, intervention: &Intervention) {
        self.break_line();
        let code = match intervention.action {
            Action::Finish => GREEN,
            Action::Escalate => RED,
            Action::StopWorker | Action::RetryWorker => YELLOW,
            _ => CYAN,
        };
        println!(
            "  {} {}  {}",
            paint(DIM, "→"),
            paint(&format!("{BOLD}{code}"), intervention.action.label()),
            paint(DIM, &intervention.reason),
        );
    }
}

/// A probability as a bar. Calibration is the point, so the bar is linear and
/// the number stays next to it.
fn bar(value: f64) -> String {
    let filled = (value.clamp(0.0, 1.0) * BAR as f64).round() as usize;
    let code = if value >= 0.8 {
        GREEN
    } else if value >= 0.5 {
        YELLOW
    } else {
        DIM
    };
    format!(
        "{}{}",
        paint(code, &"█".repeat(filled)),
        paint(DIM, &"·".repeat(BAR - filled)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_text_breaks_on_newlines_and_then_on_words() {
        let mut buffer = String::from("first\nsecond");
        assert_eq!(next_line(&mut buffer).as_deref(), Some("first"));
        assert_eq!(next_line(&mut buffer), None);
        assert_eq!(buffer, "second");

        // A single long chunk still breaks, and breaks between words.
        let mut buffer = "lorem ipsum ".repeat(30);
        let line = next_line(&mut buffer).unwrap();
        assert!(line.chars().count() <= WRAP, "{line}");
        assert!(line.ends_with("ipsum") || line.ends_with("lorem"), "{line}");
        assert!(!buffer.starts_with(' '));
    }

    #[test]
    fn a_word_longer_than_the_line_is_cut_rather_than_held_forever() {
        let mut buffer = "x".repeat(WRAP + 10);
        let line = next_line(&mut buffer).unwrap();
        assert_eq!(line.chars().count(), WRAP);
        assert_eq!(buffer.chars().count(), 10);
    }

    #[test]
    fn a_bar_is_as_long_as_the_probability() {
        // Styling aside, the cell count is fixed and the fill is linear.
        let cells = |value: f64| {
            let rendered = bar(value);
            (rendered.matches('█').count(), rendered.matches('·').count())
        };
        assert_eq!(cells(0.0), (0, BAR));
        assert_eq!(cells(1.0), (BAR, 0));
        assert_eq!(cells(0.5), (BAR / 2, BAR / 2));
        // Out-of-range input cannot overflow the bar.
        assert_eq!(cells(9.9), (BAR, 0));
    }

    #[test]
    fn every_dimension_has_a_label_that_fits_its_column() {
        for dimension in &DIMENSIONS {
            assert!(dimension.id.len() <= LABEL, "{}", dimension.id);
        }
    }
}
