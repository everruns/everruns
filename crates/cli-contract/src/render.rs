//! A stable text rendering of a clap tree.
//!
//! Two guards read this. A golden snapshot in `crates/cli` pins the contract
//! humans already type, so a refactor that re-spells a flag fails rather than
//! ships. A drift check pins the agent-facing tree to the same rendering, so
//! the two surfaces cannot diverge without a test saying so.
//!
//! The format is for diffing, not for reading: one line per argument, fields
//! in a fixed order, sorted, so a change shows up as the line that changed.

use clap::Command;

/// Render a whole command tree, one line per leaf argument.
pub fn tree(command: &Command) -> String {
    let mut lines = Vec::new();
    walk(command, "", &mut lines);
    lines.sort();
    lines.join("\n")
}

fn walk(command: &Command, path: &str, lines: &mut Vec<String>) {
    let mut children = command.get_subcommands().peekable();
    if children.peek().is_none() {
        if !path.is_empty() {
            lines.push(leaf(command, path));
        }
        return;
    }
    for child in command.get_subcommands() {
        let child_path = if path.is_empty() {
            child.get_name().to_string()
        } else {
            format!("{path} {}", child.get_name())
        };
        walk(child, &child_path, lines);
    }
}

fn leaf(command: &Command, path: &str) -> String {
    let mut parts: Vec<String> = command
        .get_arguments()
        .filter(|arg| arg.get_id() != "help" && arg.get_id() != "version")
        .map(argument)
        .collect();
    parts.sort();
    format!("{path}\t{}", parts.join(" "))
}

fn argument(arg: &clap::Arg) -> String {
    let mut spelling = String::new();
    if let Some(long) = arg.get_long() {
        spelling.push_str(&format!("--{long}"));
    }
    if let Some(short) = arg.get_short() {
        spelling.push_str(&format!("/-{short}"));
    }
    if arg.is_positional() {
        // The argument id, minus the suffix that keeps a bare-word spelling
        // distinct from its own flag. Clap's value name would do, but it
        // defaults differently for derive and builder arguments, so rendering
        // it would make this snapshot report a format change as a contract
        // change.
        let id = arg.get_id().as_str();
        let name = id.split('\u{1}').next().unwrap_or(id);
        spelling.push_str(&format!("<{name}>"));
    }
    if arg.is_required_set() {
        spelling.push('!');
    }
    if matches!(arg.get_action(), clap::ArgAction::Append) {
        spelling.push_str("...");
    }
    spelling
}

/// A line-level difference between two renderings.
///
/// Comparing the whole rendering with `assert_eq!` reports two multi-line
/// strings as two opaque blobs, which is exactly the moment a guard stops
/// being read and starts being overridden. Report the lines that moved.
pub fn diff(expected: &str, actual: &str) -> Option<String> {
    let expected: Vec<&str> = expected.trim().lines().collect();
    let actual: Vec<&str> = actual.trim().lines().collect();
    if expected == actual {
        return None;
    }

    let mut report = String::new();
    for line in &expected {
        if !actual.contains(line) {
            report.push_str(&format!("- {line}\n"));
        }
    }
    for line in &actual {
        if !expected.contains(line) {
            report.push_str(&format!("+ {line}\n"));
        }
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> clap::Command {
        clap::Command::new("root").subcommand(
            clap::Command::new("widgets").subcommand(
                clap::Command::new("list")
                    .arg(clap::Arg::new("limit").long("limit").short('l'))
                    .arg(clap::Arg::new("name").long("name").required(true)),
            ),
        )
    }

    #[test]
    fn a_leaf_renders_its_spelling_shorts_and_requirement() {
        assert_eq!(tree(&command()), "widgets list\t--limit/-l --name!");
    }

    /// The guard is only worth having if it fails, so prove it does.
    #[test]
    fn a_respelled_flag_shows_up_as_the_line_that_moved() {
        let before = tree(&command());
        let after = tree(
            &clap::Command::new("root").subcommand(
                clap::Command::new("widgets").subcommand(
                    clap::Command::new("list")
                        .arg(clap::Arg::new("limit").long("max").short('l'))
                        .arg(clap::Arg::new("name").long("name").required(true)),
                ),
            ),
        );

        let report = diff(&before, &after).expect("a re-spelled flag is a difference");
        assert!(report.contains("- widgets list\t--limit/-l"), "{report}");
        assert!(report.contains("+ widgets list\t--max/-l"), "{report}");
    }

    #[test]
    fn an_unchanged_tree_reports_nothing() {
        assert!(diff(&tree(&command()), &tree(&command())).is_none());
    }
}
