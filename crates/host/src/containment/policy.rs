//! Static checks on a shell script, before anything is spawned.
//!
//! Containment answers what a command may touch once it runs. These checks
//! answer a narrower question a kernel policy cannot: whether the command
//! visibly targets the agent process itself, and whether it is tame enough to
//! run without asking a human first.
//!
//! Every check parses the script with tree-sitter rather than matching
//! substrings. `echo kill` and `rg needle src` are ordinary commands.
//! `xargs kill < pids`, `env codex exec`, and `rg --pre ./payload` are not:
//! the last one names a read-only program and then an option that runs another
//! program. Parsing is defense in depth for a direct mistake. A deliberately
//! obscured command (`k$(echo ill) "$PID"`) is outside the trusted shape, so
//! Untrusted approval asks a person; the kernel remains the boundary when
//! containment is actually on.

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
/// The set is deliberately tiny. The program name is not enough: `rg --pre`
/// runs a preprocessor, `rg -z` runs a decompressor from `PATH`, and a bare
/// `git status` runs a repository's fsmonitor helper and hooks. Only one
/// literal command in a per-program argument shape is trusted. Anything the
/// parser cannot see through — an expansion, a glob, an assignment, a second
/// command — asks a person.
///
/// THREAT[TM-BASH-028]: under `approval=untrusted` this function is what
/// decides whether a command skips the host gate. When containment is already
/// `danger-full-access`, skipping the gate runs the command on the host.
pub fn is_trusted_read_only(script: &str) -> bool {
    let script = script.trim();
    if script.is_empty() {
        return true;
    }
    let Some(words) = literal_command(script) else {
        return false;
    };
    let Some((program, args)) = words.split_first() else {
        return true;
    };
    // A path is a different program than the one we reviewed. `./rg --pre`
    // would otherwise inherit the `rg` shape and run a workspace binary.
    if program.is_empty() || program.contains('/') {
        return false;
    }
    invocation_is_safe(program, args)
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

/// Options that do not start another program.
///
/// Unknown options are rejected. Several of these programs grow flags over
/// time; a flag we have not reviewed has to ask a person rather than inherit
/// the program's reputation.
struct SafeOptions {
    /// Short options with no value. Clustered (`-la`) and repeated (`-uuu`).
    switches: &'static str,
    /// Short options that require a value, attached (`-n5`) or following (`-n 5`).
    values: &'static str,
    /// Short options whose value, when present, is attached (`-uall`, `-f`).
    /// A following operand stays an operand, matching how these tools treat it
    /// when the value is omitted.
    optional: &'static str,
    long_switches: &'static [&'static str],
    long_values: &'static [&'static str],
    /// Present alone or as `--flag=value`. A following operand is not consumed.
    long_optional: &'static [&'static str],
}

fn invocation_is_safe(program: &str, args: &[String]) -> bool {
    match program {
        "pwd" => arguments_are_safe(args, &PWD),
        "ls" => arguments_are_safe(args, &LS),
        "cat" => arguments_are_safe(args, &CAT),
        "head" => arguments_are_safe(args, &HEAD),
        "tail" => arguments_are_safe(args, &TAIL),
        "wc" => arguments_are_safe(args, &WC),
        "rg" => arguments_are_safe(args, &RG),
        "grep" => arguments_are_safe(args, &GREP),
        "stat" => arguments_are_safe(args, &STAT),
        "file" => arguments_are_safe(args, &FILE),
        "which" => arguments_are_safe(args, &WHICH),
        // `git status` itself is not safe. `core.fsmonitor` runs a helper, and
        // refreshing the index runs `post-index-change`. Both are repository
        // config, so the trusted shape has to turn them off and refuse any
        // other `-c`, which could turn them back on.
        "git" => git_status_is_safe(args),
        _ => false,
    }
}

fn git_status_is_safe(args: &[String]) -> bool {
    let mut fsmonitor_off = false;
    let mut hooks_off = false;
    let mut index = 0;
    while index < args.len() {
        if args[index] != "-c" {
            break;
        }
        let Some(assignment) = args.get(index + 1) else {
            return false;
        };
        match assignment.as_str() {
            "core.fsmonitor=" => fsmonitor_off = true,
            "core.hooksPath=/dev/null" => hooks_off = true,
            _ => return false,
        }
        index += 2;
    }
    if !fsmonitor_off || !hooks_off {
        return false;
    }
    let Some(subcommand) = args.get(index) else {
        return false;
    };
    subcommand == "status" && arguments_are_safe(&args[index + 1..], &GIT_STATUS)
}

fn arguments_are_safe(args: &[String], options: &SafeOptions) -> bool {
    let mut index = 0;
    let mut operands_only = false;
    while index < args.len() {
        let arg = &args[index];
        if operands_only || arg == "-" || !arg.starts_with('-') {
            index += 1;
            continue;
        }
        if arg == "--" {
            operands_only = true;
            index += 1;
            continue;
        }
        let accepted = if let Some(name) = arg.strip_prefix("--") {
            long_option_is_safe(name, args, &mut index, options)
        } else {
            short_cluster_is_safe(&arg[1..], args, &mut index, options)
        };
        if !accepted {
            return false;
        }
        index += 1;
    }
    true
}

fn long_option_is_safe(
    name: &str,
    args: &[String],
    index: &mut usize,
    options: &SafeOptions,
) -> bool {
    let (flag, inline) = match name.split_once('=') {
        Some((flag, value)) => (flag, Some(value)),
        None => (name, None),
    };
    if flag.is_empty() {
        return false;
    }
    if options.long_switches.contains(&flag) {
        return inline.is_none();
    }
    if options.long_values.contains(&flag) {
        if inline.is_some() {
            return true;
        }
        *index += 1;
        return *index < args.len();
    }
    if options.long_optional.contains(&flag) {
        return true;
    }
    false
}

fn short_cluster_is_safe(
    cluster: &str,
    args: &[String],
    index: &mut usize,
    options: &SafeOptions,
) -> bool {
    if cluster.is_empty() {
        return false;
    }
    let mut chars = cluster.chars();
    while let Some(ch) = chars.next() {
        if options.switches.contains(ch) {
            continue;
        }
        if options.optional.contains(ch) {
            // The remainder is the attached value, or there is no value.
            return true;
        }
        if options.values.contains(ch) {
            if chars.next().is_some() {
                return true;
            }
            *index += 1;
            return *index < args.len();
        }
        return false;
    }
    true
}

/// One simple command, as literal argv. `None` when the script is not that:
/// more than one command, a redirect, an assignment, an expansion, a glob, or
/// a word the parser does not treat as data.
fn literal_command(script: &str) -> Option<Vec<String>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(script, None)?;
    let root = tree.root_node();
    if root.has_error() || root.kind() != "program" {
        return None;
    }

    let mut command = None;
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "comment" => {}
            "command" if command.is_none() => command = Some(child),
            _ => return None,
        }
    }
    let command = command?;
    let mut words = Vec::new();
    let mut cursor = command.walk();
    for child in command.named_children(&mut cursor) {
        words.push(literal_word(child, script)?);
    }
    Some(words)
}

/// The text a node contributes to argv, after quote removal. An expansion or
/// an unquoted glob is `None`: the shell would rewrite the argument before the
/// program saw it, and a rewritten argument can be `--pre`.
fn literal_word(node: Node<'_>, script: &str) -> Option<String> {
    match node.kind() {
        "command_name" => {
            if node.named_child_count() != 1 {
                return None;
            }
            literal_word(node.named_child(0)?, script)
        }
        "word" | "number" => {
            // A numeric base can itself be an expansion (`10#$(...)`). That is
            // not data. A bare number is: `head -n 5` parses `5` as one.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "number" {
                    return None;
                }
            }
            let text = node.utf8_text(script.as_bytes()).ok()?;
            // `\` can hide a flag (`--pre\=./payload`). `*?[` expand to
            // filenames, and `{a,b}` expands before the program runs. A file
            // named `--pre`, or `{--pre,./payload}`, would otherwise show up
            // as an operand.
            if text.contains(['\\', '*', '?', '[', ']', '{', '}']) {
                return None;
            }
            Some(text.to_string())
        }
        "raw_string" => {
            let text = node.utf8_text(script.as_bytes()).ok()?;
            let inner = text
                .strip_prefix('\'')
                .and_then(|text| text.strip_suffix('\''))?;
            Some(inner.to_string())
        }
        "string" => {
            let mut out = String::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "string_content" {
                    return None;
                }
                let text = child.utf8_text(script.as_bytes()).ok()?;
                out.push_str(&unescape_double_quoted(text)?);
            }
            Some(out)
        }
        "concatenation" => {
            let mut out = String::new();
            let mut saw_part = false;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                saw_part = true;
                out.push_str(&literal_word(child, script)?);
            }
            saw_part.then_some(out)
        }
        _ => None,
    }
}

/// Bash double quotes only consume the backslash before `$`, `` ` ``, `"`,
/// `\`, and newline. Anything else keeps the backslash.
fn unescape_double_quoted(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some(escaped @ ('$' | '`' | '"' | '\\')) => out.push(escaped),
            Some('\n') => {}
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => return None,
        }
    }
    Some(out)
}

const PWD: SafeOptions = SafeOptions {
    switches: "LP",
    values: "",
    optional: "",
    long_switches: &["logical", "physical", "help", "version"],
    long_values: &[],
    long_optional: &[],
};

const CAT: SafeOptions = SafeOptions {
    switches: "AbeEnstTuv",
    values: "",
    optional: "",
    long_switches: &[
        "show-all",
        "number-nonblank",
        "show-ends",
        "number",
        "squeeze-blank",
        "show-tabs",
        "show-nonprinting",
        "help",
        "version",
    ],
    long_values: &[],
    long_optional: &[],
};

const HEAD: SafeOptions = SafeOptions {
    switches: "qvz",
    values: "cn",
    optional: "",
    long_switches: &[
        "quiet",
        "silent",
        "verbose",
        "zero-terminated",
        "help",
        "version",
    ],
    long_values: &["bytes", "lines"],
    long_optional: &[],
};

const TAIL: SafeOptions = SafeOptions {
    switches: "Fqvz",
    values: "cn",
    optional: "f",
    long_switches: &[
        "retry",
        "quiet",
        "silent",
        "verbose",
        "zero-terminated",
        "help",
        "version",
    ],
    long_values: &[
        "bytes",
        "lines",
        "max-unchanged-stats",
        "pid",
        "sleep-interval",
    ],
    long_optional: &["follow"],
};

const WC: SafeOptions = SafeOptions {
    switches: "cmlLw",
    values: "",
    optional: "",
    long_switches: &[
        "bytes",
        "chars",
        "lines",
        "max-line-length",
        "words",
        "help",
        "version",
    ],
    long_values: &["files0-from", "total"],
    long_optional: &[],
};

const STAT: SafeOptions = SafeOptions {
    switches: "Lft",
    values: "c",
    optional: "",
    long_switches: &["dereference", "file-system", "terse", "help", "version"],
    long_values: &["cached", "format", "printf"],
    long_optional: &[],
};

const WHICH: SafeOptions = SafeOptions {
    switches: "as",
    values: "",
    optional: "",
    long_switches: &[],
    long_values: &[],
    long_optional: &[],
};

/// `file -z` / `--uncompress` looks inside compressed data. That is a
/// different program surface than identifying a file, so it stays out.
const FILE: SafeOptions = SafeOptions {
    switches: "bchiklLnp",
    values: "mefFP",
    optional: "",
    long_switches: &[
        "help",
        "version",
        "brief",
        "checking-printout",
        "apple",
        "extension",
        "mime",
        "mime-type",
        "mime-encoding",
        "keep-going",
        "list",
        "dereference",
        "no-dereference",
        "no-buffer",
        "no-pad",
        "print0",
        "preserve-date",
    ],
    long_values: &[
        "magic-file",
        "exclude",
        "exclude-quiet",
        "files-from",
        "separator",
        "parameter",
    ],
    long_optional: &[],
};

/// Reviewed against ripgrep's own help. Absent on purpose: `--pre` and
/// `--pre-glob` (run a preprocessor), `--hostname-bin` (run a program), and
/// `-z` / `--search-zip` (run a decompressor from `PATH`).
const RG: SafeOptions = SafeOptions {
    switches: "sFivxUPaSwL.ubnhHIoqcl0NpV",
    values: "efEmjgdtrTABCM",
    optional: "",
    long_switches: &[
        "case-sensitive",
        "crlf",
        "fixed-strings",
        "ignore-case",
        "invert-match",
        "line-regexp",
        "mmap",
        "multiline",
        "multiline-dotall",
        "no-unicode",
        "null-data",
        "pcre2",
        "smart-case",
        "stop-on-nonmatch",
        "text",
        "word-regexp",
        "auto-hybrid-regex",
        "no-pcre2-unicode",
        "binary",
        "follow",
        "glob-case-insensitive",
        "hidden",
        "ignore-file-case-insensitive",
        "no-ignore",
        "no-ignore-dot",
        "no-ignore-exclude",
        "no-ignore-files",
        "no-ignore-global",
        "no-ignore-parent",
        "no-ignore-vcs",
        "no-require-git",
        "one-file-system",
        "unrestricted",
        "block-buffered",
        "byte-offset",
        "column",
        "heading",
        "help",
        "include-zero",
        "line-buffered",
        "line-number",
        "no-line-number",
        "max-columns-preview",
        "no-max-columns-preview",
        "null",
        "only-matching",
        "passthru",
        "pretty",
        "quiet",
        "trim",
        "vimgrep",
        "with-filename",
        "no-filename",
        "count",
        "count-matches",
        "files-with-matches",
        "files-without-match",
        "json",
        "debug",
        "no-ignore-messages",
        "no-messages",
        "stats",
        "trace",
        "files",
        "no-config",
        "no-pre",
        "no-search-zip",
        "pcre2-version",
        "type-list",
        "version",
        "sort-files",
    ],
    long_values: &[
        "regexp",
        "file",
        "dfa-size-limit",
        "encoding",
        "engine",
        "max-count",
        "regex-size-limit",
        "threads",
        "glob",
        "iglob",
        "ignore-file",
        "cursor-ignore",
        "max-depth",
        "max-filesize",
        "type",
        "type-not",
        "type-add",
        "type-clear",
        "after-context",
        "before-context",
        "color",
        "colors",
        "context",
        "context-separator",
        "field-context-separator",
        "field-match-separator",
        "hyperlink-format",
        "max-columns",
        "path-separator",
        "replace",
        "sort",
        "sortr",
        "generate",
    ],
    long_optional: &[],
};

const GREP: SafeOptions = SafeOptions {
    // A leading digit cluster (`-5`, `-15`) is GNU grep's context shorthand.
    switches: "EFGPiwxzsvVbnhHoqaIrRLlcTZU0123456789",
    values: "efmdDABC",
    optional: "",
    long_switches: &[
        "extended-regexp",
        "fixed-strings",
        "basic-regexp",
        "perl-regexp",
        "ignore-case",
        "no-ignore-case",
        "word-regexp",
        "line-regexp",
        "null-data",
        "no-messages",
        "invert-match",
        "version",
        "help",
        "byte-offset",
        "line-number",
        "line-buffered",
        "with-filename",
        "no-filename",
        "only-matching",
        "quiet",
        "silent",
        "text",
        "recursive",
        "dereference-recursive",
        "files-without-match",
        "files-with-matches",
        "count",
        "initial-tab",
        "null",
        "no-group-separator",
        "binary",
    ],
    long_values: &[
        "regexp",
        "file",
        "max-count",
        "label",
        "binary-files",
        "directories",
        "devices",
        "include",
        "exclude",
        "exclude-from",
        "exclude-dir",
        "before-context",
        "after-context",
        "context",
        "group-separator",
    ],
    long_optional: &["color", "colour"],
};

const LS: SafeOptions = SafeOptions {
    switches: "aAbBcCdfFgGhHiklmnoOpqQrRsStUuvxX1",
    values: "ITw",
    optional: "",
    long_switches: &[
        "all",
        "almost-all",
        "author",
        "escape",
        "ignore-backups",
        "directory",
        "dired",
        "file-type",
        "full-time",
        "group-directories-first",
        "no-group",
        "human-readable",
        "si",
        "dereference-command-line",
        "dereference-command-line-symlink-to-dir",
        "inode",
        "kibibytes",
        "dereference",
        "numeric-uid-gid",
        "literal",
        "hide-control-chars",
        "show-control-chars",
        "quote-name",
        "reverse",
        "recursive",
        "size",
        "help",
        "version",
        "context",
        "group",
        "zero",
    ],
    long_values: &[
        "block-size",
        "format",
        "hide",
        "indicator-style",
        "ignore",
        "quoting-style",
        "sort",
        "time",
        "time-style",
        "tabsize",
        "width",
    ],
    long_optional: &["color", "colour", "classify", "hyperlink"],
};

const GIT_STATUS: SafeOptions = SafeOptions {
    switches: "vsbz",
    values: "",
    optional: "uM",
    long_switches: &[
        "verbose",
        "no-verbose",
        "short",
        "no-short",
        "branch",
        "no-branch",
        "show-stash",
        "no-show-stash",
        "ahead-behind",
        "no-ahead-behind",
        "long",
        "no-long",
        "null",
        "no-null",
        "no-renames",
        "renames",
    ],
    long_values: &[],
    long_optional: &[
        "porcelain",
        "no-porcelain",
        "untracked-files",
        "no-untracked-files",
        "ignored",
        "no-ignored",
        "ignore-submodules",
        "no-ignore-submodules",
        "column",
        "no-column",
        "find-renames",
        "no-find-renames",
    ],
};

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
        for command in [
            "pwd",
            "pwd -P",
            "ls -la src",
            "ls --color=auto",
            "cat file",
            "head -n 5 file",
            "tail -n 5 file",
            "wc -l file",
            "stat file",
            "file path",
            "which rg",
            "grep -n needle file",
            "grep -R needle .",
            "rg needle src",
            "rg -nH -g '*.rs' needle src",
            "rg 'a;b > c' src",
            "rg -- --pre",
            "git -c core.fsmonitor= -c core.hooksPath=/dev/null status",
            "git -c core.hooksPath=/dev/null -c core.fsmonitor= status -sb",
            "git -c core.fsmonitor= -c core.hooksPath=/dev/null status --porcelain=v2",
        ] {
            assert!(is_trusted_read_only(command), "expected trusted: {command}");
        }
        for command in [
            "rm -rf /",
            "cat a > b",
            "ls; rm x",
            "echo $(whoami)",
            "git push",
            "git status",
            "git status -sb",
            "git -c core.fsmonitor=./payload -c core.hooksPath=/dev/null status",
            "git -c core.fsmonitor= -c core.hooksPath=/dev/null -c core.fsmonitor=./payload status",
            "git -c core.fsmonitor= status",
            "git --exec-path=/tmp -c core.fsmonitor= -c core.hooksPath=/dev/null status",
            "/usr/bin/rg needle",
            "rg --pre ./payload pattern .",
            "rg --pre=./payload pattern .",
            "rg --pre\\=./payload pattern .",
            "rg $'--pre' ./payload pattern",
            "rg --pre-glob '*.rs' --pre ./payload pattern",
            "rg -z pattern .",
            "rg --search-zip pattern",
            "rg --hostname-bin ./payload pattern",
            "rg \"--pre\" ./payload pattern .",
            "rg '--pre=./payload' pattern",
            "rg {--pre,./payload} pattern",
            "rg $needle src",
            "rg needle *.rs",
            "RIPGREP_CONFIG_PATH=./cfg rg needle",
            "env rg --pre ./payload pattern",
            "file -z archive.gz",
            "file --uncompress archive.gz",
        ] {
            assert!(
                !is_trusted_read_only(command),
                "expected untrusted: {command}"
            );
        }
    }
}
