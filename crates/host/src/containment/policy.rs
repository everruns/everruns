//! Static checks on a shell script, before anything is spawned.
//!
//! Containment answers what a command may touch once it runs. These checks
//! answer a narrower question a kernel policy cannot: whether the command
//! visibly targets the agent process itself, and whether it is tame enough to
//! run without asking a human first.
//!
//! Every check parses the script with tree-sitter rather than matching
//! substrings, so `echo kill` and `rg codex src` are ordinary commands while
//! `xargs kill < pids` and `env codex exec` are not. Parsing is defense in
//! depth for direct mistakes, never the boundary: a deliberately obscured
//! command (`k$(echo ill) "$PID"`) is the kernel's problem, not the parser's.

use std::path::Path;

use tree_sitter::{Node, Parser};

/// Programs that can signal another process.
const PROCESS_CONTROL_PROGRAMS: &[&str] = &["kill", "killall", "pkill"];

/// Programs whose effects a human should see coming: process control, plus
/// nested coding agents, which spend money and edit files on their own.
const DESTRUCTIVE_PROGRAMS: &[&str] = &[
    "aider", "claude", "codex", "gemini", "kill", "killall", "pkill", "yolop",
];

/// Programs that run another program named in their arguments. Without these,
/// `xargs kill` reads as a call to `xargs`.
const EXEC_WRAPPERS: &[&str] = &[
    "command", "env", "exec", "find", "nice", "nohup", "sudo", "time", "xargs",
];

/// Commands a read-only agent may run without approval.
///
/// The set is deliberately tiny and shape-checked: any redirection, pipeline,
/// chaining, or substitution disqualifies a script outright, because the whole
/// point is that a reviewer can predict the effect from the program name.
pub fn is_trusted_read_only(script: &str) -> bool {
    if script.contains(['>', '<', ';', '&', '|', '`', '\n']) || script.contains("$(") {
        return false;
    }
    let words: Vec<&str> = script.split_whitespace().collect();
    let Some(program) = words.first().copied() else {
        return true;
    };
    match program {
        "pwd" | "ls" | "cat" | "head" | "tail" | "wc" | "rg" | "grep" | "stat" | "file"
        | "which" => true,
        "git" => words
            .get(1)
            .is_some_and(|subcommand| *subcommand == "status"),
        _ => false,
    }
}

/// Whether the script performs a destructive shell action: process control, or
/// launching another coding agent.
pub fn requires_destructive_approval(script: &str) -> bool {
    any_command_matches(script, DESTRUCTIVE_PROGRAMS)
}

/// Whether the script visibly signals the agent process running it.
///
/// THREAT[TM-BASH-021]: defense in depth for a direct mistake, not a boundary.
///
/// `host_name` is the binary's own name (`"yolop"`, `"everruns-worker"`), so a
/// `pkill -f <name>` is caught alongside a literal `kill <pid>`. Process-group
/// and broadcast targets (`0`, `-1`) count, since both reach the caller.
pub fn can_signal_host(script: &str, host_pid: u32, host_name: &str) -> bool {
    if !any_command_matches(script, PROCESS_CONTROL_PROGRAMS) {
        return false;
    }

    let host_name = host_name.to_ascii_lowercase();
    script
        .split(|character: char| !character.is_ascii_digit())
        .any(|word| word.parse::<u32>().ok() == Some(host_pid))
        || (!host_name.is_empty() && script.to_ascii_lowercase().contains(&host_name))
        || script
            .split_ascii_whitespace()
            .map(|word| word.trim_matches(|character: char| ";|&()".contains(character)))
            .any(|word| matches!(word, "0" | "-1"))
}

fn any_command_matches(script: &str, programs: &[&str]) -> bool {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .is_err()
    {
        return false;
    }
    let Some(tree) = parser.parse(script, None) else {
        return false;
    };
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        if node.kind() == "command" && command_matches(&node, script, programs) {
            return true;
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    false
}

fn command_matches(command: &Node<'_>, script: &str, programs: &[&str]) -> bool {
    let Some(name) = command.child_by_field_name("name") else {
        return false;
    };
    let Some(name) = plain_program(name, script) else {
        return false;
    };
    if programs.contains(&name.as_str()) {
        return true;
    }
    if !EXEC_WRAPPERS.contains(&name.as_str()) {
        return false;
    }

    let mut cursor = command.walk();
    command
        .children_by_field_name("argument", &mut cursor)
        .filter_map(|argument| plain_program(argument, script))
        .any(|argument| programs.contains(&argument.as_str()))
}

/// The program a node names, when it names one literally. A quoted string or
/// an expansion is data, not a program.
fn plain_program(node: Node<'_>, script: &str) -> Option<String> {
    if !matches!(node.kind(), "command_name" | "word") {
        return None;
    }
    let word = node.utf8_text(script.as_bytes()).ok()?;
    Path::new(word)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_direct_and_wrapped_destructive_commands() {
        for command in [
            "kill 1234",
            "cd /tmp && pkill -f worker",
            "env codex exec --full-auto task",
            "command claude -p task",
            "xargs kill < pids",
            "find . -exec kill {} +",
        ] {
            assert!(
                requires_destructive_approval(command),
                "expected destructive: {command}"
            );
        }
    }

    #[test]
    fn ignores_program_names_used_only_as_data() {
        for command in [
            "rg codex src",
            "printf '%s' 'kill 1234'",
            "cargo test codex",
            "echo yolop",
        ] {
            assert!(
                !requires_destructive_approval(command),
                "expected ordinary command: {command}"
            );
        }
    }

    #[test]
    fn signalling_the_host_is_recognized_by_pid_name_and_broadcast() {
        let pid = 4242;
        for command in [
            "kill 4242",
            "kill -9 4242",
            "pkill -f everruns-worker",
            "kill -TERM 0",
            "kill -1",
        ] {
            assert!(
                can_signal_host(command, pid, "everruns-worker"),
                "expected self-signal: {command}"
            );
        }
    }

    #[test]
    fn ordinary_process_control_of_other_pids_is_allowed() {
        for command in ["kill 91", "pkill -f stale-fixture", "echo kill 4242"] {
            assert!(
                !can_signal_host(command, 4242, "everruns-worker"),
                "expected allowed: {command}"
            );
        }
    }

    #[test]
    fn an_empty_host_name_never_matches_by_name() {
        // A caller with no binary name must not make every `pkill` self-directed.
        assert!(!can_signal_host("pkill -f stale", 4242, ""));
    }

    #[test]
    fn the_trusted_set_is_read_only_and_shape_checked() {
        for command in ["pwd", "ls -la src", "git status", "rg needle src"] {
            assert!(is_trusted_read_only(command), "expected trusted: {command}");
        }
        for command in [
            "rm -rf /",
            "cat a > b",
            "ls; rm x",
            "echo $(whoami)",
            "git push",
        ] {
            assert!(
                !is_trusted_read_only(command),
                "expected untrusted: {command}"
            );
        }
    }
}
