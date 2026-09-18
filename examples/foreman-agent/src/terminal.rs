//! Terminal presentation. Nothing here changes a decision.
//!
//! Two streams share one screen on purpose: the worker's own output, dimmed,
//! and the supervisor's readings cutting in while it is still talking. That
//! interleaving is the architecture, so it is what the demo shows.

use std::io::Write;
use std::sync::Mutex;

use crate::factory::Watcher;
use crate::foreman::{Assessment, DIMENSIONS, Lens};
use crate::observation::{WorkerKind, WorkerRecord, WorkerStatus};
use crate::policy::{Action, Intervention};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

const WIDTH: usize = 104;
const BAR: usize = 14;
const LABEL: usize = 24;
/// Lines shown from one shell script.
const SCRIPT_LINES: usize = 3;

fn colored() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// Clip one line to the recorded terminal width.
fn clip(line: &str) -> String {
    if line.chars().count() <= WIDTH - 4 {
        return line.to_owned();
    }
    format!("{}…", line.chars().take(WIDTH - 5).collect::<String>())
}

fn paint(code: &str, text: &str) -> String {
    if code.is_empty() || !colored() {
        return text.to_owned();
    }
    format!("{code}{text}{RESET}")
}

/// Renders a factory run as it happens.
pub struct Terminal {
    /// Column position of the worker's dimmed output, so a reading never lands
    /// halfway through one of its lines.
    column: Mutex<usize>,
}

impl Terminal {
    /// A terminal renderer at the start of a line.
    pub fn new() -> Self {
        Self {
            column: Mutex::new(0),
        }
    }

    fn break_line(&self) {
        let mut column = self.column.lock().unwrap_or_else(|e| e.into_inner());
        if *column > 0 {
            println!();
            *column = 0;
        }
    }
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
        let mut column = self.column.lock().unwrap_or_else(|e| e.into_inner());
        for chunk in delta.split_inclusive('\n') {
            if *column == 0 {
                print!("  ");
            }
            print!("{}", paint(DIM, chunk.trim_end_matches('\n')));
            if chunk.ends_with('\n') {
                println!();
                *column = 0;
            } else {
                *column += chunk.chars().count();
                if *column >= WIDTH - 4 {
                    println!();
                    *column = 0;
                }
            }
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
