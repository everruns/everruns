//! The ANSI layer every example's presentation is built from.
//!
//! Colors are escapes written unconditionally unless `NO_COLOR` is set, so a
//! piped transcript (see an example's `record.sh`) keeps them for replay under
//! `less -R`.
//!
//! Only the primitives live here. Layout — what an example chooses to show and
//! in what shape — stays in the example, because that is the part a reader is
//! meant to recognize as belonging to that agent.

use std::sync::OnceLock;

/// No styling; the default for body text.
pub const PLAIN: &str = "";
/// Reset to the terminal's own styling.
pub const RESET: &str = "\x1b[0m";
/// Bold.
pub const BOLD: &str = "\x1b[1m";
/// Dim, for anything quoted or secondary.
pub const DIM: &str = "\x1b[2m";
/// Red, for failure.
pub const RED: &str = "\x1b[31m";
/// Green, for success.
pub const GREEN: &str = "\x1b[32m";
/// Yellow, for a warning or a command.
pub const YELLOW: &str = "\x1b[33m";
/// Blue.
pub const BLUE: &str = "\x1b[34m";
/// Magenta, for section headings.
pub const MAGENTA: &str = "\x1b[35m";
/// Cyan, for titles.
pub const CYAN: &str = "\x1b[36m";

/// Longest line the demos print, chosen to fit the recorded terminal.
pub const WIDTH: usize = 104;

/// `true` unless `NO_COLOR` is set; read once.
pub fn colored() -> bool {
    static COLORED: OnceLock<bool> = OnceLock::new();
    *COLORED.get_or_init(|| std::env::var_os("NO_COLOR").is_none())
}

/// An escape sequence, or nothing when color is disabled.
pub fn sgr(code: &str) -> &str {
    if colored() { code } else { "" }
}

/// Wrap `text` in `code`, or return it unchanged when color is off.
pub fn paint(code: &str, text: &str) -> String {
    if code.is_empty() {
        return text.to_owned();
    }
    format!("{}{text}{}", sgr(code), sgr(RESET))
}

/// Clip one line to `width`, marking that something was cut.
pub fn clip_to(line: &str, width: usize) -> String {
    if line.chars().count() <= width {
        return line.to_owned();
    }
    let kept: String = line.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Clip one line to the recorded terminal width.
pub fn clip(line: &str) -> String {
    clip_to(line, WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_marks_what_it_cut_and_leaves_short_lines_alone() {
        assert_eq!(clip_to("abc", 10), "abc");
        let clipped = clip_to(&"x".repeat(20), 10);
        assert_eq!(clipped.chars().count(), 10);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn plain_text_is_never_wrapped_in_escapes() {
        assert_eq!(paint(PLAIN, "hello"), "hello");
    }
}
