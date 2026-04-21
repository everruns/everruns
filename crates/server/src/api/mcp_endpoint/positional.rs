// Positional-argument rewriter for MCP `execute` commands.
//
// bashkit's flag parser rejects positional arguments (`expected --flag, got: X`).
// LLMs naturally type `get_agent <id>` instead of `get_agent --id <id>`,
// producing retry loops. This module pre-rewrites the raw command string at
// statement boundaries so the bash interpreter sees `get_agent --id <id>`.
//
// The rewrite is conservative: it only fires at statement-start positions
// (start of string, after `;`, `|`, `&`, `\n`, `(`, `{`, comments), only when
// the command name is followed by at least one space or tab, and only when
// the next token is a single positional (not already `--flag`). Quoted
// regions and backslash escapes are preserved as-is. See EVE-323.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Positional map shared across all `tool_execute` calls. Built once from the
/// inventory registry, which is fixed at compile time.
static POSITIONAL_MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

/// Return the cached positional map. The map key is a command name; the value
/// is the flag name to insert before the first positional argument (usually
/// `"id"`).
pub fn positional_map() -> &'static HashMap<&'static str, &'static str> {
    POSITIONAL_MAP.get_or_init(build_positional_map)
}

fn build_positional_map() -> HashMap<&'static str, &'static str> {
    let mut map: HashMap<&'static str, &'static str> = HashMap::new();

    // Inventory-registered commands that opt in via CommandDescriptor::positional_arg.
    for desc in inventory::iter::<crate::domains::common::CommandDescriptor> {
        if let Some(flag) = (desc.positional_arg)() {
            map.insert((desc.meta)().name, flag);
        }
    }

    map
}

/// Rewrite `<cmd> <arg>` as `<cmd> --<positional> <arg>` at statement
/// boundaries for commands in `map`. Commands not in `map` pass through.
/// Quoted regions, backslash escapes, and comments are preserved verbatim
/// (including multi-byte UTF-8 sequences).
pub fn rewrite(input: &str, map: &HashMap<&'static str, &'static str>) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + 32);
    let mut i = 0;
    let mut at_stmt_start = true;

    while i < bytes.len() {
        let b = bytes[i];

        if at_stmt_start && is_name_start(b) {
            // Read the command name.
            let name_start = i;
            while i < bytes.len() && is_name_cont(bytes[i]) {
                i += 1;
            }
            let name_end = i;
            out.extend_from_slice(&bytes[name_start..name_end]);

            if let Some(&positional) =
                map.get(std::str::from_utf8(&bytes[name_start..name_end]).unwrap_or(""))
            {
                // Capture whitespace between command and next token.
                let ws_start = i;
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                let ws_len = i - ws_start;

                // Only rewrite if the command name is actually separated from
                // the next token by whitespace. `get_agent'abc'` and
                // `get_agent$ID` are bash token concatenations that produce a
                // different command name; leave them alone.
                let should_rewrite =
                    ws_len > 0 && i < bytes.len() && is_positional_value_start(bytes[i]);

                if should_rewrite {
                    out.push(b' ');
                    out.extend_from_slice(b"--");
                    out.extend_from_slice(positional.as_bytes());
                    out.extend_from_slice(&bytes[ws_start..i]);
                } else {
                    out.extend_from_slice(&bytes[ws_start..i]);
                }
            }
            at_stmt_start = false;
            continue;
        }

        match b {
            b'\'' => {
                // Single-quoted string: no escapes inside, copy verbatim to
                // the next '.
                out.push(b'\'');
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    out.push(bytes[i]);
                    i += 1;
                }
                if i < bytes.len() {
                    out.push(b'\'');
                    i += 1;
                }
                at_stmt_start = false;
            }
            b'"' => {
                // Double-quoted string: backslash escapes one byte.
                out.push(b'"');
                i += 1;
                while i < bytes.len() {
                    let c = bytes[i];
                    out.push(c);
                    i += 1;
                    if c == b'\\' && i < bytes.len() {
                        out.push(bytes[i]);
                        i += 1;
                        continue;
                    }
                    if c == b'"' {
                        break;
                    }
                }
                at_stmt_start = false;
            }
            b'\\' => {
                // Unquoted backslash: consume the next byte as literal.
                out.push(b'\\');
                i += 1;
                if i < bytes.len() {
                    out.push(bytes[i]);
                    i += 1;
                }
                at_stmt_start = false;
            }
            b'#' if at_stmt_start_prev(bytes, i) => {
                // Comment: copy through end-of-line; newline handling happens
                // on the next iteration and keeps stmt_start=true.
                while i < bytes.len() && bytes[i] != b'\n' {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'#' => {
                // Mid-token `#` (e.g. `get_agent a#b`) is a literal.
                out.push(b'#');
                i += 1;
            }
            b'`' => {
                // Legacy backtick command substitution. Treat as a quoted
                // region with no nested rewrite. `$(...)` is the preferred
                // form and does get rewritten inside.
                out.push(b'`');
                i += 1;
                while i < bytes.len() && bytes[i] != b'`' {
                    let c = bytes[i];
                    out.push(c);
                    i += 1;
                    if c == b'\\' && i < bytes.len() {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                if i < bytes.len() {
                    out.push(b'`');
                    i += 1;
                }
                at_stmt_start = false;
            }
            b';' | b'|' | b'&' | b'\n' | b'(' | b'{' => {
                out.push(b);
                i += 1;
                at_stmt_start = true;
            }
            b' ' | b'\t' => {
                out.push(b);
                i += 1;
                // Whitespace doesn't reset at_stmt_start (it preserves the
                // previous state so leading whitespace still counts as
                // statement start).
            }
            _ => {
                out.push(b);
                i += 1;
                at_stmt_start = false;
            }
        }
    }

    // Safety: the input is a valid UTF-8 `&str`; we only append ASCII bytes
    // (`--<flag>` where <flag> is a compile-time ASCII identifier) and copy
    // arbitrary input bytes verbatim. The concatenation preserves UTF-8
    // because valid UTF-8 sub-slices joined with ASCII stay valid UTF-8.
    debug_assert!(std::str::from_utf8(&out).is_ok());
    unsafe { String::from_utf8_unchecked(out) }
}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_name_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A byte is a positional-value start when it begins a non-flag, non-boundary
/// token. `-` (any dash-prefixed token) is excluded so existing `--flag`
/// usage is preserved. Quotes count because the quoted content is the value.
fn is_positional_value_start(b: u8) -> bool {
    !matches!(
        b,
        b'-' | b';' | b'|' | b'&' | b'\n' | b')' | b'}' | b'#' | b' ' | b'\t'
    )
}

/// True when the byte at `i-1` is whitespace or a boundary — i.e. `#` at
/// position `i` starts a comment rather than sitting inside a token.
fn at_stmt_start_prev(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    matches!(
        bytes[i - 1],
        b' ' | b'\t' | b';' | b'|' | b'&' | b'\n' | b'(' | b'{'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_map() -> HashMap<&'static str, &'static str> {
        let mut m = HashMap::new();
        m.insert("get_agent", "id");
        m.insert("delete_agent", "id");
        m.insert("get_session", "session_id");
        m
    }

    #[test]
    fn simple_positional() {
        let m = test_map();
        assert_eq!(
            rewrite("get_agent agent_123", &m),
            "get_agent --id agent_123"
        );
    }

    #[test]
    fn already_flag_form_preserved() {
        let m = test_map();
        assert_eq!(
            rewrite("get_agent --id agent_123", &m),
            "get_agent --id agent_123"
        );
    }

    #[test]
    fn short_flag_not_rewritten() {
        let m = test_map();
        // `-h` starts with `-` so we don't treat it as positional; bashkit
        // will reject it as "expected --flag" (the user's bug, not ours).
        assert_eq!(rewrite("get_agent -h", &m), "get_agent -h");
    }

    #[test]
    fn unknown_command_passthrough() {
        let m = test_map();
        assert_eq!(rewrite("list_sessions foo", &m), "list_sessions foo");
    }

    #[test]
    fn statement_boundary_semicolon() {
        let m = test_map();
        assert_eq!(
            rewrite("get_agent a1; delete_agent a2", &m),
            "get_agent --id a1; delete_agent --id a2"
        );
    }

    #[test]
    fn statement_boundary_pipe() {
        let m = test_map();
        assert_eq!(
            rewrite("get_agent a1 | jq .name", &m),
            "get_agent --id a1 | jq .name"
        );
    }

    #[test]
    fn statement_boundary_newline() {
        let m = test_map();
        let input = "get_agent a1\ndelete_agent a2\n";
        let expected = "get_agent --id a1\ndelete_agent --id a2\n";
        assert_eq!(rewrite(input, &m), expected);
    }

    #[test]
    fn statement_boundary_andand() {
        let m = test_map();
        assert_eq!(
            rewrite("get_agent a1 && get_agent a2", &m),
            "get_agent --id a1 && get_agent --id a2"
        );
    }

    #[test]
    fn statement_boundary_subshell() {
        let m = test_map();
        assert_eq!(rewrite("x=$(get_agent a1)", &m), "x=$(get_agent --id a1)");
    }

    #[test]
    fn quoted_arg_is_rewritten() {
        let m = test_map();
        assert_eq!(
            rewrite("get_agent 'agent_xyz'", &m),
            "get_agent --id 'agent_xyz'"
        );
        assert_eq!(
            rewrite("get_agent \"agent_xyz\"", &m),
            "get_agent --id \"agent_xyz\""
        );
    }

    #[test]
    fn command_name_inside_quotes_untouched() {
        let m = test_map();
        // `echo "get_agent foo"` — the string inside quotes is a payload, not
        // a statement start. Our tokenizer enters quote mode on `"` so the
        // inner `get_agent foo` is not rewritten.
        assert_eq!(
            rewrite("echo \"get_agent foo\"", &m),
            "echo \"get_agent foo\""
        );
    }

    #[test]
    fn command_name_after_pipe_inside_quotes() {
        let m = test_map();
        // A `|` inside quotes is not a statement boundary.
        assert_eq!(
            rewrite("echo 'a|get_agent b' | cat", &m),
            "echo 'a|get_agent b' | cat"
        );
    }

    #[test]
    fn preserves_whitespace() {
        let m = test_map();
        assert_eq!(
            rewrite("get_agent   agent_1", &m),
            "get_agent --id   agent_1"
        );
    }

    #[test]
    fn trailing_command_with_no_arg_unchanged() {
        let m = test_map();
        // No positional follows — leave it to bashkit to produce its own
        // "required arg missing" error.
        assert_eq!(rewrite("get_agent", &m), "get_agent");
        assert_eq!(rewrite("get_agent   ", &m), "get_agent   ");
        assert_eq!(rewrite("get_agent  ;", &m), "get_agent  ;");
    }

    #[test]
    fn variable_expansion_arg() {
        let m = test_map();
        assert_eq!(rewrite("get_agent $AGENT", &m), "get_agent --id $AGENT");
    }

    #[test]
    fn multiple_positional_args_only_first_rewritten() {
        let m = test_map();
        // Second positional is left alone — bashkit rejects it. We don't try
        // to be smart: the common case is a single id.
        assert_eq!(rewrite("get_agent a b", &m), "get_agent --id a b");
    }

    #[test]
    fn escaped_space_in_arg() {
        let m = test_map();
        assert_eq!(rewrite("get_agent a\\ b", &m), "get_agent --id a\\ b");
    }

    #[test]
    fn comment_line_skipped() {
        let m = test_map();
        assert_eq!(
            rewrite("# get_agent a\nget_agent b", &m),
            "# get_agent a\nget_agent --id b"
        );
    }

    #[test]
    fn leading_whitespace_preserved() {
        let m = test_map();
        assert_eq!(rewrite("  get_agent a", &m), "  get_agent --id a");
    }

    #[test]
    fn empty_input() {
        let m = test_map();
        assert_eq!(rewrite("", &m), "");
    }

    #[test]
    fn different_positional_name() {
        let m = test_map();
        assert_eq!(
            rewrite("get_session sess_1", &m),
            "get_session --session_id sess_1"
        );
    }

    #[test]
    fn no_whitespace_between_cmd_and_value_unchanged() {
        // bash concatenation: `get_agent'abc'` tokenizes as the command name
        // `get_agentabc`, not `get_agent` followed by `abc`. Likewise
        // `get_agent$ID` is `get_agent<expanded>`. Rewriting these would
        // change the command name bash ultimately sees, so the rewriter
        // requires at least one whitespace separator.
        let m = test_map();
        assert_eq!(rewrite("get_agent'abc'", &m), "get_agent'abc'");
        assert_eq!(rewrite("get_agent$ID", &m), "get_agent$ID");
        assert_eq!(rewrite("get_agent\"a\"", &m), "get_agent\"a\"");
    }

    #[test]
    fn utf8_payload_preserved() {
        // The rewriter must not corrupt multi-byte UTF-8 sequences in
        // quoted strings or comments. `é` is two bytes (0xC3 0xA9).
        let m = test_map();
        assert_eq!(
            rewrite("echo 'é' ; get_agent a1", &m),
            "echo 'é' ; get_agent --id a1"
        );
        assert_eq!(rewrite("get_agent 'id-é-1'", &m), "get_agent --id 'id-é-1'");
    }

    #[test]
    fn build_positional_map_contains_known_commands() {
        let map = positional_map();
        // Inventory commands with id: at least get_agent, get_capability.
        assert_eq!(map.get("get_agent").copied(), Some("id"));
        assert_eq!(map.get("get_capability").copied(), Some("id"));
        // MCP-only inventory commands still expose their positional path args.
        assert_eq!(map.get("get_session").copied(), Some("session_id"));
        assert_eq!(map.get("get_model").copied(), Some("id"));
        // List endpoints have no path params — must not be present.
        assert!(!map.contains_key("list_agents"));
        assert!(!map.contains_key("list_sessions"));
        // Multi-path-param ops (e.g. delete_app_channel and get_session_file)
        // must not be present: positional ordering would be ambiguous.
        assert!(!map.contains_key("delete_app_channel"));
        assert!(!map.contains_key("get_session_file"));
    }

    #[test]
    fn inventory_contains_unique_positional_entries() {
        let map = build_positional_map();
        assert_eq!(map.get("get_agent").copied(), Some("id"));
    }
}
