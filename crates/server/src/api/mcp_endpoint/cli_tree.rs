// Noun-verb command tree for the scripted `everruns` surface.
//
// The catalog's flat builtin names (`list_agents`) are the wire identity and
// stay the thing that executes. This module adds the tree spelling
// (`everruns agents list`) on top of them, so one command string works in a
// session's shell, in an MCP `execute` script, and — once the external binary
// moves onto it — in a terminal.
//
// It is a source rewrite rather than a builtin of its own because bashkit's
// `ToolArgs` carries only parsed `--flag` params: a builtin can never see the
// bare words `agents list`. Rewriting at statement boundaries reuses every
// downstream guarantee unchanged — the per-command schema, flag coercion,
// policy checks in `Command::run`, and error sanitization are the same code
// the flat name already goes through. The tree is a spelling, not a second
// execution path.
//
// Help is bounded by the tree's own shape: the root lists nouns, a node lists
// its children, a leaf renders its flags via `catalog::bash_usage`. That is
// what makes `--help` affordable here when the flat 312-command namespace had
// to forbid it (see `knowledge/execution/capabilities.md`, Platform tools).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::domains::common::{CliRoute, CommandDescriptor};

/// Builtin the rewriter emits for any help or usage request.
pub const HELP_BUILTIN: &str = "everruns_help";

/// Root token that introduces a tree invocation.
pub const ROOT: &str = "everruns";

/// One resolved leaf: the tree spelling and the flat command it runs.
#[derive(Debug, Clone)]
pub struct Leaf {
    pub command: &'static str,
    pub description: &'static str,
    pub route: CliRoute,
}

/// The assembled tree. Nodes are keyed by their full path so lookup is a
/// single map hit; children are derived for help rendering.
#[derive(Debug, Default)]
pub struct CliTree {
    /// "agents list" -> leaf
    leaves: BTreeMap<String, Leaf>,
    /// "agents" -> ["agents versions", ...] non-leaf child paths
    nodes: BTreeMap<String, Vec<String>>,
}

impl CliTree {
    pub fn leaf(&self, path: &str) -> Option<&Leaf> {
        self.leaves.get(path)
    }

    pub fn is_node(&self, path: &str) -> bool {
        self.nodes.contains_key(path)
    }

    /// Direct children of a node path, as (last segment, description) pairs.
    /// The empty path returns the tree's top-level nouns.
    pub fn children(&self, path: &str) -> Vec<(String, String)> {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path} ")
        };
        let mut seen: BTreeMap<String, String> = BTreeMap::new();

        for (spelling, leaf) in &self.leaves {
            let Some(rest) = spelling.strip_prefix(prefix.as_str()) else {
                continue;
            };
            if path.is_empty() && prefix.is_empty() && spelling.is_empty() {
                continue;
            }
            let mut parts = rest.split(' ');
            let Some(head) = parts.next() else { continue };
            if parts.next().is_none() {
                // Direct leaf child: its own description.
                seen.insert(head.to_string(), leaf.description.to_string());
            } else {
                let child_path = if path.is_empty() {
                    head.to_string()
                } else {
                    format!("{path} {head}")
                };
                seen.entry(head.to_string()).or_insert_with(|| {
                    node_about(&child_path)
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| format!("{head} commands"))
                });
            }
        }

        seen.into_iter().collect()
    }

    fn insert(&mut self, leaf: Leaf) {
        let spelling = leaf.route.spelling();
        // Register every ancestor path as a node so `everruns agents --help`
        // and `everruns agents versions --help` both resolve.
        let segments: Vec<&str> = leaf.route.path.to_vec();
        for depth in 1..=segments.len() {
            let node = segments[..depth].join(" ");
            self.nodes.entry(node).or_default();
        }
        self.leaves.insert(spelling, leaf);
    }
}

/// One-line summaries for tree nodes.
///
/// A node is a grouping, not a command, so it has no `CommandMeta` to borrow a
/// description from. Without this the root renders "agents — agents commands",
/// which costs prompt budget and teaches nothing. Keyed by full node path.
const NODE_ABOUT: &[(&str, &str)] = &[
    (
        "agents",
        "Agent definitions, versions, and their configuration.",
    ),
    (
        "agents versions",
        "Immutable snapshots of an agent, and rollback between them.",
    ),
    (
        "mcp-servers",
        "Registered MCP servers available to agents and sessions.",
    ),
    (
        "sessions",
        "Running and archived sessions, their state and participants.",
    ),
    ("sessions participants", "Who is attached to a session."),
    ("skills", "Skill packages and their content."),
];

fn node_about(path: &str) -> Option<&'static str> {
    NODE_ABOUT
        .iter()
        .find(|(node, _)| *node == path)
        .map(|(_, about)| *about)
}

static TREE: OnceLock<CliTree> = OnceLock::new();

/// The process-wide tree, built once from inventory.
///
/// Feature gating is deliberately *not* applied here: the tree is shared
/// across orgs, and a feature-disabled command still fails closed at
/// dispatch through its own policy and the catalog's exposure rules. Gating
/// the spelling as well would make the same script legal in one org and a
/// parse error in another.
pub fn tree() -> &'static CliTree {
    TREE.get_or_init(build_tree)
}

fn build_tree() -> CliTree {
    let mut tree = CliTree::default();
    for desc in inventory::iter::<CommandDescriptor> {
        let Some(route) = (desc.cli)() else {
            continue;
        };
        let meta = (desc.meta)();
        tree.insert(Leaf {
            command: meta.name,
            description: meta.description,
            route,
        });
    }
    tree
}

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

            if name == ROOT {
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
        return (help_call(&path, None), end);
    }
    if let Some(leaf) = tree.leaf(&path) {
        return (leaf.command.to_string(), i);
    }
    if path.is_empty() || tree.is_node(&path) {
        return (help_call(&path, None), i);
    }

    // Unresolved: report against the deepest node that does exist, so the
    // error names real neighbours rather than the whole tree.
    let mut known: Vec<&str> = path.split(' ').collect();
    let unknown = known.pop().unwrap_or_default().to_string();
    let mut parent = known.join(" ");
    while !parent.is_empty() && !tree.is_node(&parent) {
        let mut segments: Vec<&str> = parent.split(' ').collect();
        segments.pop();
        parent = segments.join(" ");
    }
    (help_call(&parent, Some(&unknown)), i)
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

/// Build the help builtin invocation. Path words are `[a-z0-9-]` by
/// construction, so single quotes cannot be broken out of.
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

/// Render help for a tree path.
///
/// Every response is bounded by the tree's shape rather than by a cap: the
/// root lists nouns, a node lists its children, a leaf lists its own flags.
pub fn render_help(
    tree: &CliTree,
    path: &str,
    unknown: Option<&str>,
    // (wire command name, display name) -> usage block. The display name is
    // what the caller typed, so the usage line reads back the tree spelling
    // rather than the flat alias they did not use.
    usage_for: impl Fn(&str, &str) -> String,
) -> Result<String, String> {
    let path = path.trim();

    if let Some(word) = unknown.filter(|value| !value.is_empty()) {
        let scope = if path.is_empty() {
            ROOT.to_string()
        } else {
            format!("{ROOT} {path}")
        };
        let mut text = format!("unknown command `{word}` under `{scope}`\n\n");
        text.push_str(&render_children(tree, path));
        return Err(text);
    }

    if let Some(leaf) = tree.leaf(path) {
        let mut text = format!("{ROOT} {path}\n  {}\n\n", leaf.description);
        text.push_str(&usage_for(leaf.command, &format!("{ROOT} {path}")));
        if !leaf.route.examples.is_empty() {
            text.push_str("\nExamples:\n");
            for example in leaf.route.examples {
                text.push_str(&format!("  {example}\n"));
            }
        }
        text.push_str(&format!("\nWire name: {}\n", leaf.command));
        return Ok(text);
    }

    if path.is_empty() || tree.is_node(path) {
        return Ok(render_children(tree, path));
    }

    Err(format!(
        "unknown command `{ROOT} {path}`\n\n{}",
        render_children(tree, "")
    ))
}

fn render_children(tree: &CliTree, path: &str) -> String {
    let children = tree.children(path);
    let scope = if path.is_empty() {
        ROOT.to_string()
    } else {
        format!("{ROOT} {path}")
    };

    if children.is_empty() {
        return format!("{scope}\n  no commands available\n");
    }

    let width = children
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0)
        .min(24);

    let mut text = match node_about(path) {
        Some(about) => {
            format!("{scope}\n  {about}\n\nUsage: {scope} <command> [--flags]\n\nCommands:\n")
        }
        None => format!("Usage: {scope} <command> [--flags]\n\nCommands:\n"),
    };
    for (name, description) in children {
        let summary = first_sentence(&description);
        text.push_str(&format!("  {name:<width$}  {summary}\n"));
    }
    text.push_str(&format!("\nRun `{scope} <command> --help` for flags.\n"));
    text
}

/// Help lines stay one line each: catalog descriptions are written for a JSON
/// catalog and often run several sentences.
fn first_sentence(description: &str) -> String {
    let trimmed = description.trim();
    let end = trimmed
        .find(". ")
        .map(|index| index + 1)
        .unwrap_or(trimmed.len());
    let mut line = trimmed[..end].trim().to_string();
    if line.len() > 96 {
        line.truncate(93);
        line.push_str("...");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_usage(_wire: &str, display: &str) -> String {
        format!("Usage: {display} [--flags]\n")
    }

    fn rw(input: &str) -> String {
        rewrite(input, tree())
    }

    #[test]
    fn rewrites_a_leaf_to_its_flat_command() {
        assert_eq!(
            rw("everruns agents list --limit 10"),
            "list_agents --limit 10"
        );
        assert_eq!(rw("everruns mcp-servers list"), "list_mcp_servers");
    }

    #[test]
    fn rewrites_a_nested_leaf() {
        assert_eq!(
            rw("everruns agents versions list --agent_id agt_1"),
            "list_agent_versions --agent_id agt_1"
        );
        assert_eq!(
            rw("everruns sessions participants list --session_id ses_1"),
            "list_session_participants --session_id ses_1"
        );
    }

    #[test]
    fn leaf_wins_over_a_longer_walk() {
        // `agents list` resolves at two words; trailing words are flags/values,
        // never more path.
        assert_eq!(rw("everruns agents list extra"), "list_agents extra");
    }

    #[test]
    fn flat_names_still_pass_through_untouched() {
        assert_eq!(rw("list_agents --limit 10"), "list_agents --limit 10");
        assert_eq!(
            rw("get_agent agt_1 | jq .name"),
            "get_agent agt_1 | jq .name"
        );
    }

    #[test]
    fn composes_with_pipelines_and_statements() {
        assert_eq!(
            rw("everruns agents list | jq '.data[].name'"),
            "list_agents | jq '.data[].name'"
        );
        assert_eq!(
            rw("everruns skills list; everruns agents list"),
            "list_skills; list_agents"
        );
    }

    #[test]
    fn does_not_rewrite_inside_quotes() {
        assert_eq!(
            rw("echo 'everruns agents list'"),
            "echo 'everruns agents list'"
        );
        assert_eq!(
            rw("echo \"everruns agents list\""),
            "echo \"everruns agents list\""
        );
    }

    #[test]
    fn does_not_rewrite_a_different_command_name() {
        assert_eq!(rw("everrunsx agents list"), "everrunsx agents list");
    }

    #[test]
    fn help_requests_become_the_help_builtin() {
        assert_eq!(rw("everruns --help"), "everruns_help --path ''");
        assert_eq!(
            rw("everruns agents --help"),
            "everruns_help --path 'agents'"
        );
        assert_eq!(
            rw("everruns agents list --help"),
            "everruns_help --path 'agents list'"
        );
        assert_eq!(rw("everruns"), "everruns_help --path ''");
    }

    #[test]
    fn a_node_without_a_verb_shows_its_children() {
        assert_eq!(rw("everruns agents"), "everruns_help --path 'agents'");
        assert_eq!(
            rw("everruns agents versions"),
            "everruns_help --path 'agents versions'"
        );
    }

    #[test]
    fn an_unknown_verb_reports_against_the_nearest_real_node() {
        assert_eq!(
            rw("everruns agents lst"),
            "everruns_help --path 'agents' --unknown 'lst'"
        );
        assert_eq!(
            rw("everruns nope"),
            "everruns_help --path '' --unknown 'nope'"
        );
    }

    /// Entry names in a rendered help block, in order.
    fn listed_commands(text: &str) -> Vec<String> {
        text.lines()
            .skip_while(|line| !line.starts_with("Commands:"))
            .skip(1)
            .take_while(|line| line.starts_with("  "))
            .filter_map(|line| line.split_whitespace().next().map(ToOwned::to_owned))
            .collect()
    }

    #[test]
    fn root_help_lists_nouns_only() {
        // The root is bounded because it lists nouns, never the verbs beneath
        // them: this is what makes `--help` affordable where the flat
        // 312-command namespace had to forbid it.
        let text = render_help(tree(), "", None, stub_usage).expect("root help");
        assert_eq!(
            listed_commands(&text),
            vec!["agents", "mcp-servers", "sessions", "skills"],
            "{text}"
        );
    }

    #[test]
    fn node_help_lists_direct_children_only() {
        let listed = listed_commands(&render_help(tree(), "agents", None, stub_usage).unwrap());
        assert!(listed.contains(&"list".to_string()), "{listed:?}");
        assert!(listed.contains(&"versions".to_string()), "{listed:?}");
        // A grandchild verb belongs to `agents versions`, not to `agents`.
        assert!(!listed.contains(&"set-default".to_string()), "{listed:?}");
    }

    #[test]
    fn leaf_help_carries_flags_examples_and_the_wire_name() {
        let text = render_help(tree(), "agents list", None, stub_usage).expect("leaf help");
        // The usage line reads back what the caller typed, not the alias.
        assert!(text.contains("Usage: everruns agents list"), "{text}");
        assert!(text.contains("Examples:"), "{text}");
        assert!(text.contains("everruns agents list --search"), "{text}");
        assert!(text.contains("Wire name: list_agents"), "{text}");
    }

    #[test]
    fn unknown_verb_help_is_an_error_naming_real_neighbours() {
        let error =
            render_help(tree(), "agents", Some("lst"), stub_usage).expect_err("should be an error");
        assert!(error.contains("unknown command `lst`"), "{error}");
        assert!(error.contains("list"), "{error}");
    }

    #[test]
    fn soft_and_hard_delete_stay_separate_verbs() {
        // They carry different policies (MANAGE vs DANGEROUS), so the tree
        // must not collapse them behind one spelling plus a flag.
        assert_eq!(rw("everruns agents delete agt_1"), "delete_agent agt_1");
        assert_eq!(rw("everruns agents destroy agt_1"), "destroy_agent agt_1");
    }

    #[test]
    fn every_declared_route_is_unique_and_reachable() {
        let mut seen = std::collections::BTreeSet::new();
        for desc in inventory::iter::<CommandDescriptor> {
            let Some(route) = (desc.cli)() else { continue };
            let spelling = route.spelling();
            assert!(
                seen.insert(spelling.clone()),
                "duplicate tree spelling: {spelling}"
            );
            let leaf = tree()
                .leaf(&spelling)
                .unwrap_or_else(|| panic!("{spelling} is declared but not reachable"));
            assert_eq!(leaf.command, (desc.meta)().name);
        }
        assert!(
            seen.len() >= 50,
            "tranche should be declared: {}",
            seen.len()
        );
    }

    #[test]
    fn no_route_shadows_a_flat_command_name() {
        // A tree path segment that collides with a flat builtin name would make
        // the rewrite ambiguous at statement start.
        let flat: std::collections::BTreeSet<&str> = inventory::iter::<CommandDescriptor>
            .into_iter()
            .map(|desc| (desc.meta)().name)
            .collect();
        for desc in inventory::iter::<CommandDescriptor> {
            let Some(route) = (desc.cli)() else { continue };
            for segment in route.path {
                assert!(
                    !flat.contains(segment),
                    "tree segment `{segment}` collides with a flat command name"
                );
            }
        }
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::api::mcp_endpoint::catalog::bash_usage;

    /// The leaf help has to show real, typeable flags: the whole reason a tree
    /// can afford `--help` is that each leaf is small. If this ever renders an
    /// empty flag list the surface silently regresses to "guess the schema".
    #[test]
    fn leaf_usage_renders_real_flags_under_the_tree_spelling() {
        let schema = inventory::iter::<CommandDescriptor>
            .into_iter()
            .find(|desc| (desc.meta)().name == "create_mcp_server")
            .map(|desc| (desc.param_schema)())
            .expect("create_mcp_server is registered");

        let text = render_help(tree(), "mcp-servers create", None, |_wire, display| {
            bash_usage(display, &schema)
        })
        .expect("leaf help");

        assert!(text.contains("everruns mcp-servers create"), "{text}");
        assert!(text.contains("--name"), "{text}");
        assert!(text.contains("--url"), "{text}");
        assert!(text.contains("Wire name: create_mcp_server"), "{text}");
    }
}
