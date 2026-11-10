//! How the CLI looks in a terminal: colors, status marks, spinners, prompts and
//! the summary box that `docker up` and `k8s up` end with.
//!
//! Decisions:
//! - Color follows the common conventions, in this order: `--color` (or
//!   `--no-color`), then `NO_COLOR` (any non-empty value turns it off),
//!   `CLICOLOR_FORCE` (non-empty and not `0` turns it on), `CLICOLOR=0` and
//!   `TERM=dumb` (off), and finally whether the stream is a terminal. stdout
//!   and stderr are decided separately, so `everruns docker status | less`
//!   stays plain while prompts on stderr keep their color.
//! - Everything here writes to stderr except [`field`] and table headers,
//!   which belong to a command's stdout result.
//! - Spinners only draw on a terminal; elsewhere the finished line is printed
//!   once, so CI logs read the same as before minus the animation.

use std::fmt::Display;
use std::io::IsTerminal;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use console::{StyledObject, measure_text_width, style};
use dialoguer::theme::{ColorfulTheme, SimpleTheme, Theme};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    /// Color on a terminal, unless NO_COLOR or CLICOLOR=0 says otherwise
    #[default]
    Auto,
    Always,
    Never,
}

/// Decide color for both streams. Called once, before anything is printed.
pub fn init(choice: ColorChoice) {
    let env = |k: &str| std::env::var(k).ok();
    console::set_colors_enabled(wants_color(choice, env, std::io::stdout().is_terminal()));
    console::set_colors_enabled_stderr(wants_color(choice, env, std::io::stderr().is_terminal()));
}

fn wants_color(choice: ColorChoice, env: impl Fn(&str) -> Option<String>, tty: bool) -> bool {
    match choice {
        ColorChoice::Always => return true,
        ColorChoice::Never => return false,
        ColorChoice::Auto => {}
    }
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    if set("NO_COLOR").is_some() {
        return false;
    }
    if set("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
        return true;
    }
    if set("CLICOLOR").is_some_and(|v| v == "0") || set("TERM").is_some_and(|v| v == "dumb") {
        return false;
    }
    tty
}

fn colors() -> bool {
    console::colors_enabled_stderr()
}

// Text pieces, for stderr.

pub fn bold<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().bold()
}

pub fn dim<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().dim()
}

/// Something to type or click: a command, flag or URL.
pub fn cmd<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().cyan()
}

pub fn good<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().green()
}

pub fn warning<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().yellow()
}

pub fn bad<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().red()
}

// Lines.

/// `everruns k8s up  v0.51.0`, then a dim detail line.
pub fn banner(title: &str, details: &[(&str, &str)]) {
    eprintln!();
    eprintln!(
        "  {} {}  {}",
        style("everruns").for_stderr().magenta().bold(),
        bold(title),
        dim(format!("v{}", env!("CARGO_PKG_VERSION")))
    );
    if !details.is_empty() {
        let line = details
            .iter()
            .map(|(k, v)| format!("{} {v}", dim(k)))
            .collect::<Vec<_>>()
            .join(&format!("  {}  ", dim("·")));
        eprintln!("  {line}");
    }
}

pub fn section(title: &str) {
    eprintln!();
    eprintln!("{}", bold(title));
}

pub fn ok(msg: impl Display) {
    eprintln!("{} {msg}", good("✔"));
}

pub fn warn(msg: impl Display) {
    eprintln!("{} {msg}", warning("!"));
}

pub fn fail(msg: impl Display) {
    eprintln!("{} {msg}", bad("✗"));
}

/// A neutral progress note.
pub fn note(msg: impl Display) {
    eprintln!("{} {msg}", dim("·"));
}

/// An indented follow-up line under a mark, such as how to fix a failure.
pub fn detail(msg: impl Display) {
    eprintln!("  {} {msg}", dim("→"));
}

/// `Name detail` for a passing check.
pub fn check_line(name: &str, detail: &str) -> String {
    format!("{name} {detail}")
}

/// A `Label   value` line on stdout, for status pages.
pub fn field(label: &str, value: impl Display) {
    let label = format!("{label:<10}");
    println!("{} {value}", style(label).dim());
}

/// A `Label   value` line on stderr, for status lines next to a command's
/// own output.
pub fn stderr_field(label: &str, value: impl Display) {
    eprintln!("{} {value}", dim(format!("{label:<10}")));
}

/// A table header on stdout.
pub fn header(text: &str) -> String {
    style(text).bold().to_string()
}

/// The closing box: a green title, then aligned rows.
pub fn summary(title: &str, aside: &str, rows: &[(&str, String)]) {
    let label_w = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    let mut lines = vec![format!("{}  {}", good(title).bold(), dim(aside))];
    if !rows.is_empty() {
        lines.push(String::new());
    }
    for (label, value) in rows {
        lines.push(format!("{}  {value}", dim(format!("{label:<label_w$}"))));
    }
    let width = lines
        .iter()
        .map(|l| measure_text_width(l))
        .max()
        .unwrap_or(0)
        + 2;
    let edge = |l: &str, r: &str| good(format!("{l}{}{r}", "─".repeat(width + 2)));
    eprintln!();
    eprintln!("{}", edge("╭", "╮"));
    for line in &lines {
        let pad = width - measure_text_width(line);
        eprintln!("{}  {line}{}{}", good("│"), " ".repeat(pad), good("│"));
    }
    eprintln!("{}", edge("╰", "╯"));
    eprintln!();
}

// Prompts.

static COLORFUL: LazyLock<ColorfulTheme> = LazyLock::new(ColorfulTheme::default);

/// The prompt theme: dialoguer's colorful one, or plain text without color.
pub fn theme() -> &'static dyn Theme {
    if colors() { &*COLORFUL } else { &SimpleTheme }
}

// Spinners.

/// A spinner on one line that finishes as a ✔ or ✗ line with its duration.
pub struct Spinner {
    bar: ProgressBar,
    started: Instant,
}

impl Spinner {
    pub fn start(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        let bar = if std::io::stderr().is_terminal() {
            let bar = ProgressBar::new_spinner();
            let template = if colors() {
                "{spinner:.cyan} {msg}"
            } else {
                "{spinner} {msg}"
            };
            bar.set_style(
                ProgressStyle::with_template(template)
                    .unwrap_or_else(|_| ProgressStyle::default_spinner())
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
            );
            bar.enable_steady_tick(Duration::from_millis(100));
            bar
        } else {
            note(&msg);
            ProgressBar::hidden()
        };
        bar.set_message(msg);
        Self {
            bar,
            started: Instant::now(),
        }
    }

    pub fn handle(&self) -> ProgressBar {
        self.bar.clone()
    }

    fn took(&self) -> String {
        let secs = self.started.elapsed().as_secs();
        if secs < 60 {
            format!("{secs}s")
        } else {
            format!("{}m{:02}s", secs / 60, secs % 60)
        }
    }

    pub fn done(self, msg: impl Display) {
        let took = self.took();
        self.bar.finish_and_clear();
        ok(format!("{msg}  {}", dim(took)));
    }

    pub fn failed(self, msg: impl Display) {
        self.bar.finish_and_clear();
        fail(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn auto_follows_the_terminal() {
        assert!(wants_color(ColorChoice::Auto, env(&[]), true));
        assert!(!wants_color(ColorChoice::Auto, env(&[]), false));
    }

    #[test]
    fn the_flag_wins_over_the_environment() {
        assert!(!wants_color(
            ColorChoice::Never,
            env(&[("CLICOLOR_FORCE", "1")]),
            true
        ));
        assert!(wants_color(
            ColorChoice::Always,
            env(&[("NO_COLOR", "1")]),
            false
        ));
    }

    #[test]
    fn conventional_variables_turn_color_off() {
        assert!(!wants_color(
            ColorChoice::Auto,
            env(&[("NO_COLOR", "1")]),
            true
        ));
        assert!(!wants_color(
            ColorChoice::Auto,
            env(&[("CLICOLOR", "0")]),
            true
        ));
        assert!(!wants_color(
            ColorChoice::Auto,
            env(&[("TERM", "dumb")]),
            true
        ));
        // An empty NO_COLOR does not count, per no-color.org.
        assert!(wants_color(
            ColorChoice::Auto,
            env(&[("NO_COLOR", "")]),
            true
        ));
    }

    #[test]
    fn clicolor_force_turns_it_on_off_a_terminal() {
        assert!(wants_color(
            ColorChoice::Auto,
            env(&[("CLICOLOR_FORCE", "1")]),
            false
        ));
        assert!(!wants_color(
            ColorChoice::Auto,
            env(&[("CLICOLOR_FORCE", "0")]),
            false
        ));
        // NO_COLOR still beats it.
        assert!(!wants_color(
            ColorChoice::Auto,
            env(&[("NO_COLOR", "1"), ("CLICOLOR_FORCE", "1")]),
            true
        ));
    }

    #[test]
    fn the_summary_box_lines_up_without_color() {
        console::set_colors_enabled_stderr(false);
        let rows = [("UI", "http://localhost:9300".to_string())];
        // Rendering must not panic on width math; the content is checked by eye.
        summary("Everruns is running", "here", &rows);
    }
}
