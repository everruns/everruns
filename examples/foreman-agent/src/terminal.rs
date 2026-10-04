//! Compact worker and supervisor timeline. Presentation never changes policy.

use std::io::Write;
use std::sync::Mutex;

use everruns_example_demo::style::{
    BOLD, CYAN, DIM, GREEN, MAGENTA, RED, WIDTH, YELLOW, clip_to, paint,
};

use crate::factory::Watcher;
use crate::foreman::{Assessment, DIMENSIONS};
use crate::observation::{TestRun, WorkerKind, WorkerRecord, WorkerStatus};
use crate::policy::{Action, Intervention};

/// Lines shown from one shell script.
const SCRIPT_LINES: usize = 3;
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

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
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
            WorkerKind::Verifier => "independent verifier · READ-ONLY workspace",
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

    fn worker_tool(&self, worker_id: &str, tool: &str, script: &str) {
        self.break_line();
        println!(
            "  {} {} · {}",
            paint(CYAN, worker_id),
            paint(DIM, "tool"),
            paint(YELLOW, tool)
        );
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

    fn assessed(&self, iteration: usize, assessment: &Assessment, active: Option<&WorkerRecord>) {
        self.break_line();
        let status = active.map_or_else(
            || "workers idle".to_owned(),
            |worker| {
                format!(
                    "observed {} RUNNING · {:.0}s · {} tools",
                    worker.id,
                    worker.elapsed().as_secs_f64(),
                    worker.tool_calls
                )
            },
        );
        println!(
            "\n  {}  {}",
            paint(
                &format!("{BOLD}{MAGENTA}"),
                &format!("SUPERVISOR · reading {iteration}")
            ),
            paint(CYAN, &status)
        );
        for row in DIMENSIONS.chunks(3) {
            let cells = row
                .iter()
                .map(|dimension| {
                    let value = assessment.value(dimension.id);
                    let risk = matches!(
                        dimension.id,
                        "worker_stuck" | "work_off_track" | "needs_human"
                    );
                    let code = if value >= 0.8 {
                        if risk { RED } else { GREEN }
                    } else {
                        DIM
                    };
                    format!(
                        "{:LABEL$} {}",
                        dimension.id,
                        paint(&format!("{BOLD}{code}"), &format!("{value:.2}"))
                    )
                })
                .collect::<Vec<_>>();
            println!("    {}", cells.join("  "));
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

#[cfg(test)]
#[path = "../tests/unit/terminal.rs"]
mod tests;
