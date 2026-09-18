//! Rewriting a tree spelling into its flat wire name at statement boundaries.
//!
//! For a `ScriptedTool` host, which parses `--flag` pairs before a builtin runs
//! and never surfaces bare words.

use super::builtin::split_at_unknown;
use super::*;

/// How far the rewriter will walk bare words after `everruns` before giving
/// up. The deepest declared path today is three segments plus a verb; the cap
/// stops a pathological line from being scanned indefinitely.
const MAX_PATH_WORDS: usize = 5;

/// Rewrite tree invocations into their flat command names.
///
/// `everruns agents list --limit 10` becomes `list_agents --limit 10`, so the
/// existing parser, schema, and dispatch see exactly what they see today.
/// Anything that does not resolve becomes a `everruns_help` invocation that
/// prints the live children and exits non-zero, so a wrong guess answers with
/// the real options instead of a bare parse error.
///
/// Conservative in the same way `positional::rewrite` is: it fires only at
/// statement-start positions, only on the bare word `everruns`, and it stops
/// consuming at the first token that is quoted, expanded, or flag-like.
pub fn rewrite(input: &str, tree: &CliTree) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + 32);
    let mut i = 0;
    let mut at_stmt_start = true;

    while i < bytes.len() {
        let b = bytes[i];

        if at_stmt_start && is_name_start(b) {
            let name_start = i;
            while i < bytes.len() && is_name_cont(bytes[i]) {
                i += 1;
            }
            let name = std::str::from_utf8(&bytes[name_start..i]).unwrap_or("");

            if name == tree.root() {
                let (replacement, consumed) = resolve_invocation(bytes, i, tree);
                out.extend_from_slice(replacement.as_bytes());
                i = consumed;
                at_stmt_start = false;
                continue;
            }

            out.extend_from_slice(&bytes[name_start..i]);
            at_stmt_start = false;
            continue;
        }

        out.push(b);
        at_stmt_start = match b {
            b';' | b'|' | b'&' | b'\n' | b'(' | b'{' => true,
            b' ' | b'\t' => at_stmt_start,
            _ => false,
        };
        i += 1;

        // Copy quoted regions and escapes verbatim so a tree word inside a
        // string is never rewritten.
        match b {
            b'\'' => {
                while i < bytes.len() && bytes[i] != b'\'' {
                    out.push(bytes[i]);
                    i += 1;
                }
                if i < bytes.len() {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'"' => {
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        out.push(bytes[i]);
                        i += 1;
                    }
                    out.push(bytes[i]);
                    i += 1;
                }
                if i < bytes.len() {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'\\' if i < bytes.len() => {
                out.push(bytes[i]);
                i += 1;
            }
            _ => {}
        }
    }

    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

/// Consume the bare words after `everruns` and return the replacement text
/// plus the new cursor position.
fn resolve_invocation(bytes: &[u8], mut i: usize, tree: &CliTree) -> (String, usize) {
    let mut words: Vec<String> = Vec::new();
    let mut wants_help = false;
    let mut cursor = i;

    while words.len() < MAX_PATH_WORDS {
        let ws_start = cursor;
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        if cursor == ws_start {
            break; // No separator: `everrunsfoo` is a different command.
        }
        if cursor >= bytes.len() || !is_word_char(bytes[cursor]) {
            break;
        }
        let word_start = cursor;
        while cursor < bytes.len() && is_word_char(bytes[cursor]) {
            cursor += 1;
        }
        let word = String::from_utf8_lossy(&bytes[word_start..cursor]).into_owned();

        // A flag ends path consumption. `--help` anywhere in the path portion
        // turns the invocation into a help request for what was read so far.
        if word == "--help" || word == "-h" {
            wants_help = true;
            i = cursor;
            break;
        }

        words.push(word);
        i = cursor;

        if tree.leaf(&words.join(" ")).is_some() {
            break; // Longest match is a leaf; the rest is flags.
        }
    }

    let path = words.join(" ");

    // `--help` may sit anywhere in the command's flags, not just in the path
    // words: `everruns agents list --help` resolves a leaf first and would
    // otherwise pass `--help` down to a builtin that has no such flag. Help
    // wins over execution, and consumes the whole simple command so no
    // stray flags reach the help builtin.
    let end = statement_end(bytes, i);
    if wants_help || contains_help_flag(bytes, i, end) {
        let (scope, unknown) = split_at_unknown(tree, &path);
        return (help_call(&scope, unknown.as_deref()), end);
    }
    if let Some(leaf) = tree.leaf(&path) {
        return (leaf.command.to_string(), i);
    }
    if path.is_empty() || tree.is_node(&path) {
        return (help_call(&path, None), i);
    }

    // Unresolved: report against the deepest node that does exist, so the
    // error names real neighbours rather than the whole tree.
    let (scope, unknown) = split_at_unknown(tree, &path);
    (help_call(&scope, unknown.as_deref()), i)
}

/// End of the current simple command: the next unquoted statement separator.
fn statement_end(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b';' | b'|' | b'&' | b'\n' | b')' | b'}' => return i,
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'\\' => i += 1,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// Whether a bare `--help` / `-h` token appears in `[from, end)`. Quoted
/// regions are skipped so `--flag '--help'` is a value, not a help request.
fn contains_help_flag(bytes: &[u8], mut i: usize, end: usize) -> bool {
    let end = end.min(bytes.len());
    while i < end {
        match bytes[i] {
            b'\'' => {
                i += 1;
                while i < end && bytes[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            }
            b'"' => {
                i += 1;
                while i < end && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' => {
                let start = i;
                while i < end && !matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                let token = &bytes[start..i];
                if token == b"--help" || token == b"-h" {
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// Build the help builtin invocation.
///
// THREAT[TM-BASH-011]: this is the one place the rewriter emits shell text
// built from caller input, so a word carrying a quote would break out of the
// single-quoted argument and inject a command. Two independent guards: the
// tokenizer only accepts `[A-Za-z0-9_-]` as word characters, so a quote can
// never be captured, and `sanitize` strips anything else regardless. Covered by
// `quoting_cannot_be_broken_out_of`.
fn help_call(path: &str, unknown: Option<&str>) -> String {
    let safe_path = sanitize(path);
    match unknown {
        Some(word) => format!(
            "{HELP_BUILTIN} --path '{safe_path}' --unknown '{}'",
            sanitize(word)
        ),
        None => format!("{HELP_BUILTIN} --path '{safe_path}'"),
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .collect()
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')
}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_name_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Security-relevant properties of the rewrite, kept separate so a change that
/// weakens one is obvious in the diff.
#[cfg(test)]
mod rewrite_safety_tests {
    use super::*;

    struct OneCommand;

    #[async_trait]
    impl CliCommandSource for OneCommand {
        fn specs(&self) -> Vec<CliCommandSpec> {
            vec![CliCommandSpec {
                wire_name: "list_widgets".into(),
                description: "List widgets.".into(),
                path: vec!["widgets".into()],
                verb: "list".into(),
                command: clap::Command::new("list").color(clap::ColorChoice::Never),
            }]
        }
        async fn dispatch(
            &self,
            _wire: &str,
            _matches: clap::ArgMatches,
        ) -> Result<String, String> {
            Ok("{}".into())
        }
    }

    fn tree() -> CliTree {
        CliTree::from_source(&OneCommand)
    }

    #[test]
    fn the_synthesizer_emits_only_sanitized_words() {
        // THREAT[TM-BASH-011]: `help_call` is the only place the rewriter
        // synthesizes shell text from caller input, so the invariant is tested
        // where it lives rather than by slicing the combined output. Anything
        // that could end a quoted token must not survive into the argument.
        for hostile in [
            "widgets'; rm -rf /",
            "$(id)",
            "`id`",
            "a\nb",
            "a\"b",
            "a\\b",
            "a;b|c&d",
        ] {
            let call = help_call(hostile, Some(hostile));
            let quoted: String = call.chars().filter(|c| *c == '\'').collect();
            assert_eq!(
                quoted.len(),
                4,
                "expected exactly two quoted arguments in {call:?}"
            );
            for bad in [';', '`', '&', '|', '$', '"', '\\', '\n', '\''] {
                assert!(
                    !call.split('\'').nth(1).is_some_and(|arg| arg.contains(bad)),
                    "{bad:?} survived into {call:?}"
                );
            }
        }
    }

    #[test]
    fn the_tokenizer_never_captures_a_metacharacter() {
        // The first of the two guards: a hostile byte is not a word character,
        // so it is never consumed as a path word in the first place.
        for bad in [b'\'', b'"', b';', b'|', b'&', b'$', b'`', b'\\', b'\n'] {
            assert!(
                !is_word_char(bad),
                "{} is treated as a word char",
                bad as char
            );
        }
    }

    #[test]
    fn the_caller_s_own_text_is_passed_through_verbatim() {
        // The corollary of the above: the rewriter replaces only the
        // `everruns <words>` prefix and must not rewrite, escape, or drop the
        // rest, or a legitimate script would change meaning.
        let tree = tree();
        let out = rewrite("everruns widgets list | jq -r '.total' > out.txt", &tree);
        assert_eq!(out, "list_widgets | jq -r '.total' > out.txt");
    }

    #[test]
    fn a_tree_word_inside_a_string_is_left_alone() {
        let tree = tree();
        assert_eq!(
            rewrite("echo 'everruns widgets list'", &tree),
            "echo 'everruns widgets list'"
        );
    }

    #[test]
    fn the_rewrite_only_ever_emits_a_declared_wire_name() {
        // The rewrite cannot conjure a command: every leaf maps to a name the
        // source declared, so it can never name a builtin the host did not
        // choose to expose. Whether that name is *executable* remains the
        // host's decision (a read-only toolset registers no mutating builtin,
        // and an unregistered name is simply not found).
        let tree = tree();
        assert_eq!(rewrite("everruns widgets list", &tree), "list_widgets");
        assert!(rewrite("everruns widgets create", &tree).starts_with(HELP_BUILTIN));
    }
}
