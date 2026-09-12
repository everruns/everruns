// The `everruns` noun-verb command tree.
//
// One grammar, several hosts. A command's canonical identity stays its flat
// wire name (`list_agents`); this adds the spelling a caller types
// (`everruns agents list`) and the bounded help that makes such a surface
// discoverable at all.
//
// The tree is deliberately transport-neutral. Two adapters exist because two
// hosts differ in what they can see, not because the grammar differs:
//
//   * A plain bash tool hands a builtin its raw argv, so [`CliBuiltin`] parses
//     the tree directly.
//   * A `ScriptedTool` host parses `--flag` pairs before a builtin runs and
//     never surfaces bare words, so [`rewrite`] converts the tree spelling
//     into the flat name at statement boundaries before the interpreter sees
//     it. Everything downstream — schema, coercion, authorization, error
//     handling — is then the same code the flat name already went through.
//
// Where the commands come from is the host's business: [`CliCommandSource`]
// is the seam. A server backs it with its domain-command catalog; a Framework
// application backs it with whatever it owns. A host that provides no source
// gets no tree and advertises none, rather than promising a surface it cannot
// serve.
//
// Help is bounded by the tree's shape rather than by a cap: the root lists
// nouns, a node lists its children, a leaf renders its own flags. That is what
// makes `--help` affordable where a flat namespace of hundreds of commands has
// to forbid it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use bashkit::ExecResult;
use serde_json::Value;

/// Builtin the rewriter emits for any help or usage request.
pub const HELP_BUILTIN: &str = "everruns_help";

/// Root token that introduces a tree invocation.
pub const ROOT: &str = "everruns";

/// Where a command sits in the command tree.
///
/// Opt-in by construction: a command joins the tree only by declaring one, so
/// internal plumbing cannot leak into an agent-facing surface by being
/// written.
///
/// `path` is a slice rather than a single noun because flat command names hide
/// a hierarchy: `list_session_participants` is `sessions participants list`.
/// Deriving that by string surgery is wrong for exactly the irregular names
/// that matter, so the shape is declared, not inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliRoute {
    /// Noun path from the tree root, e.g. `["agents"]` or `["agents", "versions"]`.
    pub path: &'static [&'static str],
    /// Leaf verb, e.g. `"list"`.
    pub verb: &'static str,
    /// Complete, runnable invocations rendered under the leaf's help.
    ///
    /// Callers re-probe `--help` when argument forms appear only on leaves, so
    /// an example carries real flags rather than restating the syntax.
    pub examples: &'static [&'static str],
}

impl CliRoute {
    pub const fn new(path: &'static [&'static str], verb: &'static str) -> Self {
        Self {
            path,
            verb,
            examples: &[],
        }
    }

    pub const fn with_examples(mut self, examples: &'static [&'static str]) -> Self {
        self.examples = examples;
        self
    }

    /// Space-joined spelling, e.g. `"agents versions list"`.
    pub fn spelling(&self) -> String {
        let mut parts = self.path.to_vec();
        parts.push(self.verb);
        parts.join(" ")
    }
}

/// One command offered to the tree by a [`CliCommandSource`].
#[derive(Debug, Clone)]
pub struct CliCommandSpec {
    /// Canonical identity passed back to [`CliCommandSource::dispatch`].
    pub wire_name: String,
    /// One-line summary, rendered in help.
    pub description: String,
    pub route: CliRoute,
}

/// Where a host's commands come from, and how one runs.
///
/// The tree owns grammar, help, and dispatch *routing*; a source owns the
/// commands themselves and their authorization. Nothing here assumes a server:
/// a Framework application implements this over its own operations.
#[async_trait]
pub trait CliCommandSource: Send + Sync {
    /// Commands to expose in the tree.
    fn specs(&self) -> Vec<CliCommandSpec>;

    /// The word that introduces an invocation: `everruns agents list`.
    ///
    /// A host's own operations are its own, so the token that names them is
    /// the host's too. The hosted product keeps the default; a Framework
    /// application administering invoices has no reason to spell them under
    /// someone else's brand.
    fn root(&self) -> &str {
        ROOT
    }

    /// One-line summaries for grouping nodes, keyed by full node path
    /// (`"agents"`, `"agents versions"`).
    ///
    /// A node is a grouping, not a command, so it has no description to
    /// borrow. Without these the root renders "agents — agents commands",
    /// which spends prompt budget and teaches nothing.
    fn node_about(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Usage block for one command, given the spelling the caller typed.
    fn usage(&self, wire_name: &str, display_name: &str) -> String {
        let _ = wire_name;
        format!("Usage: {display_name} [--flags]\n")
    }

    /// Run a command. `params` is the parsed argument object.
    async fn dispatch(&self, wire_name: &str, params: Value) -> Result<String, String>;
}

/// One resolved leaf: the tree spelling and the command it runs.
#[derive(Debug, Clone)]
pub struct Leaf {
    pub command: String,
    pub description: String,
    pub route: CliRoute,
}

/// The assembled tree. Nodes are keyed by their full path so lookup is a
/// single map hit; children are derived for help rendering.
#[derive(Debug)]
pub struct CliTree {
    /// Token that introduces an invocation, from the source that built it.
    root: String,
    node_about: BTreeMap<String, String>,
    /// "agents list" -> leaf
    leaves: BTreeMap<String, Leaf>,
    /// "agents" -> ["agents versions", ...] non-leaf child paths
    nodes: BTreeMap<String, Vec<String>>,
}

impl Default for CliTree {
    fn default() -> Self {
        Self {
            root: ROOT.to_string(),
            node_about: BTreeMap::new(),
            leaves: BTreeMap::new(),
            nodes: BTreeMap::new(),
        }
    }
}

impl CliTree {
    /// The token that introduces an invocation in this tree.
    pub fn root(&self) -> &str {
        &self.root
    }

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
                seen.insert(head.to_string(), leaf.description.clone());
            } else {
                let child_path = if path.is_empty() {
                    head.to_string()
                } else {
                    format!("{path} {head}")
                };
                seen.entry(head.to_string()).or_insert_with(|| {
                    self.about(&child_path)
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| format!("{head} commands"))
                });
            }
        }

        seen.into_iter().collect()
    }

    /// Build a tree from a source's declared commands.
    pub fn from_source(source: &dyn CliCommandSource) -> Self {
        let mut tree = Self {
            root: source.root().to_string(),
            node_about: source.node_about().into_iter().collect(),
            ..Self::default()
        };
        for spec in source.specs() {
            tree.insert(Leaf {
                command: spec.wire_name,
                description: spec.description,
                route: spec.route,
            });
        }
        tree
    }

    fn about(&self, path: &str) -> Option<&str> {
        self.node_about.get(path).map(String::as_str)
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
    let root = tree.root();

    if let Some(word) = unknown.filter(|value| !value.is_empty()) {
        let scope = if path.is_empty() {
            root.to_string()
        } else {
            format!("{root} {path}")
        };
        let mut text = format!("unknown command `{word}` under `{scope}`\n\n");
        text.push_str(&render_children(tree, path));
        return Err(text);
    }

    if let Some(leaf) = tree.leaf(path) {
        let mut text = format!("{root} {path}\n  {}\n\n", leaf.description);
        text.push_str(&usage_for(&leaf.command, &format!("{root} {path}")));
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
        "unknown command `{root} {path}`\n\n{}",
        render_children(tree, "")
    ))
}

fn render_children(tree: &CliTree, path: &str) -> String {
    let children = tree.children(path);
    let root = tree.root();
    let scope = if path.is_empty() {
        root.to_string()
    } else {
        format!("{root} {path}")
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

    let mut text = match tree.about(path) {
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

// ============================================================================
// Bash-tool adapter
// ============================================================================

/// The `everruns` builtin for a plain bash tool.
///
/// Unlike a `ScriptedTool` host, a builtin here receives raw argv, so the tree
/// is walked directly and no source rewriting is involved. Flags after the
/// resolved leaf are parsed into the parameter object the source dispatches
/// on: `--name x` becomes `{"name": "x"}`, a bare `--flag` becomes `true`, and
/// a lone leading value is bound to the leaf's positional field when the
/// source declares one through its usage.
pub struct CliBuiltin {
    source: Arc<dyn CliCommandSource>,
    tree: CliTree,
}

impl CliBuiltin {
    pub fn new(source: Arc<dyn CliCommandSource>) -> Self {
        let tree = CliTree::from_source(source.as_ref());
        Self { source, tree }
    }

    pub fn tree(&self) -> &CliTree {
        &self.tree
    }

    /// Resolve argv into either a command to run or help to print.
    fn plan(&self, args: &[String]) -> CliPlan {
        let wants_help = args.iter().any(|arg| arg == "--help" || arg == "-h");

        let mut path: Vec<String> = Vec::new();
        let mut rest = args.len();
        for (index, arg) in args.iter().enumerate() {
            if arg.starts_with('-') {
                rest = index;
                break;
            }
            path.push(arg.clone());
            if self.tree.leaf(&path.join(" ")).is_some() {
                rest = index + 1;
                break;
            }
            rest = index + 1;
        }

        let spelling = path.join(" ");

        if wants_help {
            // Help beats execution: a caller asking what a command does must
            // never accidentally run it. It must still fail when the path is
            // not real, or a typo renders as a working help page.
            let (path, unknown) = split_at_unknown(&self.tree, &spelling);
            return CliPlan::Help { path, unknown };
        }
        if let Some(leaf) = self.tree.leaf(&spelling) {
            return CliPlan::Run {
                wire_name: leaf.command.clone(),
                args: args[rest.min(args.len())..].to_vec(),
            };
        }
        if spelling.is_empty() || self.tree.is_node(&spelling) {
            return CliPlan::Help {
                path: spelling,
                unknown: None,
            };
        }

        let (path, unknown) = split_at_unknown(&self.tree, &spelling);
        CliPlan::Help { path, unknown }
    }

    /// Render help, or run the command and return its output.
    pub async fn run(&self, args: &[String]) -> Result<String, String> {
        match self.plan(args) {
            CliPlan::Help { path, unknown } => {
                render_help(&self.tree, &path, unknown.as_deref(), |wire, display| {
                    self.source.usage(wire, display)
                })
            }
            CliPlan::Run { wire_name, args } => {
                let params = parse_flags(&args)?;
                self.source.dispatch(&wire_name, params).await
            }
        }
    }
}

enum CliPlan {
    Help {
        path: String,
        unknown: Option<String>,
    },
    Run {
        wire_name: String,
        args: Vec<String>,
    },
}

/// Split a typed path into the deepest prefix the tree knows and the first
/// segment that does not resolve under it.
///
/// Walking forward matters. Collapsing to the nearest known ancestor and
/// blaming whatever word was left over reports the *last* token, so
/// `gadgets resize thing 4` becomes "unknown command `4`" when `gadgets` is
/// the word that does not exist. The first unresolvable segment is the one
/// the caller got wrong; everything after it was never reachable.
fn split_at_unknown(tree: &CliTree, path: &str) -> (String, Option<String>) {
    let mut known: Vec<&str> = Vec::new();

    for segment in path.split(' ').filter(|segment| !segment.is_empty()) {
        let mut candidate = known.clone();
        candidate.push(segment);
        let joined = candidate.join(" ");
        if tree.is_node(&joined) || tree.leaf(&joined).is_some() {
            known = candidate;
        } else {
            return (known.join(" "), Some(segment.to_string()));
        }
    }

    (known.join(" "), None)
}

/// Parse `--flag value` pairs into a JSON object.
///
/// Values stay strings unless they are unambiguous JSON scalars or documents:
/// the source validates against its own schema, and guessing a type here would
/// turn a version string like `1.20` into a float.
fn parse_flags(args: &[String]) -> Result<Value, String> {
    let mut object = serde_json::Map::new();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        let Some(name) = arg.strip_prefix("--") else {
            return Err(format!(
                "unexpected argument `{arg}`: pass values as `--flag value`"
            ));
        };
        if name.is_empty() {
            return Err("unexpected `--`".to_string());
        }

        // `--flag=value` and `--flag value` are both accepted.
        if let Some((key, value)) = name.split_once('=') {
            object.insert(key.to_string(), scalar(value));
            index += 1;
            continue;
        }

        match args.get(index + 1) {
            Some(value) if !value.starts_with("--") => {
                object.insert(name.to_string(), scalar(value));
                index += 2;
            }
            // A trailing or flag-followed switch is a boolean.
            _ => {
                object.insert(name.to_string(), Value::Bool(true));
                index += 1;
            }
        }
    }

    Ok(Value::Object(object))
}

fn scalar(value: &str) -> Value {
    let trimmed = value.trim();
    if matches!(trimmed, "true" | "false") {
        return Value::Bool(trimmed == "true");
    }
    // JSON documents passed as text (`--capabilities '["bash"]'`) reach the
    // source structured, matching how the scripted host delivers them.
    if (trimmed.starts_with('[') || trimmed.starts_with('{'))
        && let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
    {
        return parsed;
    }
    Value::String(value.to_string())
}

/// Host-supplied command source, carried on `ToolContext` extensions.
///
/// The extension bag keys by concrete type, so the trait object needs a named
/// wrapper. A session whose host inserts one gets the `everruns` builtin; a
/// session whose host does not gets no builtin and no prompt claim about it.
#[derive(Clone)]
pub struct CliCommandSourceHandle(pub Arc<dyn CliCommandSource>);

impl std::fmt::Debug for CliCommandSourceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliCommandSourceHandle").finish()
    }
}

/// bashkit builtin wrapper around [`CliBuiltin`].
pub struct EverrunsBuiltin {
    inner: CliBuiltin,
}

impl EverrunsBuiltin {
    pub fn new(source: Arc<dyn CliCommandSource>) -> Self {
        Self {
            inner: CliBuiltin::new(source),
        }
    }

    /// The token this builtin answers to.
    pub fn root(&self) -> &str {
        self.inner.tree().root()
    }

    /// Comma-joined top-level nouns, for a host that wants to name them in
    /// its own prompt contribution.
    pub fn nouns(&self) -> Vec<String> {
        self.inner
            .tree()
            .children("")
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }
}

#[async_trait]
impl bashkit::Builtin for EverrunsBuiltin {
    async fn execute(&self, ctx: bashkit::BuiltinContext<'_>) -> bashkit::Result<ExecResult> {
        let args = ctx.args.to_vec();
        match self.inner.run(&args).await {
            Ok(output) => Ok(ExecResult::ok(ensure_trailing_newline(output))),
            // Exit non-zero so an unusable command never reads as success:
            // help printed for an unknown verb is a failure, not output.
            Err(error) => Ok(ExecResult::err(ensure_trailing_newline(error), 1)),
        }
    }

    /// A CLI does not advertise itself the way a tool schema does, so the
    /// interpreter's hint is the pointer that makes it findable.
    ///
    /// The trait wants a `&'static str` and a builtin is rebuilt per
    /// execution, so this cannot enumerate the live nouns without leaking on
    /// every call. It names the shape and sends the caller to `--help`, which
    /// is generated from the tree and therefore never goes stale.
    fn llm_hint(&self) -> Option<&'static str> {
        Some(interned_hint(self.root()))
    }
}

/// Hint text for one root token, interned for the process lifetime.
///
/// The trait wants a `&'static str` and a builtin is rebuilt per execution, so
/// the hint cannot be built per call without leaking on every one. Interning
/// by root bounds the leak to the number of distinct root tokens a process
/// uses, which is one for any host that is not embedding several trees.
fn interned_hint(root: &str) -> &'static str {
    static HINTS: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

    if root == ROOT {
        // The overwhelmingly common case allocates nothing.
        return "everruns <noun> <verb> [--flags] administers this deployment. \
                Run `everruns --help` for the nouns and `everruns <noun> --help` for its verbs.";
    }

    let hints = HINTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut hints = match hints.lock() {
        Ok(guard) => guard,
        // A poisoned lock must not take the shell down over a hint string.
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(hint) = hints.get(root) {
        return hint;
    }
    let hint: &'static str = Box::leak(
        format!(
            "{root} <noun> <verb> [--flags] administers this deployment. \
             Run `{root} --help` for the nouns and `{root} <noun> --help` for its verbs."
        )
        .into_boxed_str(),
    );
    hints.insert(root.to_string(), hint);
    hint
}

fn ensure_trailing_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source standing in for whatever a host owns: two nouns, one nested,
    /// enough to exercise the tree without a server.
    struct TestSource;

    const LIST: CliRoute = CliRoute::new(&["widgets"], "list");
    const CREATE: CliRoute = CliRoute::new(&["widgets"], "create");
    const PARTS: CliRoute = CliRoute::new(&["widgets", "parts"], "list");

    #[async_trait]
    impl CliCommandSource for TestSource {
        fn specs(&self) -> Vec<CliCommandSpec> {
            vec![
                CliCommandSpec {
                    wire_name: "list_widgets".into(),
                    description: "List widgets.".into(),
                    route: LIST.with_examples(&["everruns widgets list --limit 5"]),
                },
                CliCommandSpec {
                    wire_name: "create_widget".into(),
                    description: "Create a widget.".into(),
                    route: CREATE,
                },
                CliCommandSpec {
                    wire_name: "list_widget_parts".into(),
                    description: "List the parts of a widget.".into(),
                    route: PARTS,
                },
            ]
        }

        fn node_about(&self) -> Vec<(String, String)> {
            vec![("widgets".to_string(), "The widgets.".to_string())]
        }

        async fn dispatch(&self, wire_name: &str, params: Value) -> Result<String, String> {
            if wire_name == "create_widget" && params.get("name").is_none() {
                return Err("create_widget: missing --name".to_string());
            }
            Ok(serde_json::json!({ "ran": wire_name, "params": params }).to_string())
        }
    }

    fn builtin() -> CliBuiltin {
        CliBuiltin::new(Arc::new(TestSource))
    }

    fn argv(line: &str) -> Vec<String> {
        line.split_whitespace().map(ToOwned::to_owned).collect()
    }

    #[tokio::test]
    async fn runs_a_leaf_and_passes_its_flags_through() {
        let out = builtin()
            .run(&argv("widgets list --limit 5"))
            .await
            .expect("runs");
        assert!(out.contains("\"ran\":\"list_widgets\""), "{out}");
        assert!(out.contains("\"limit\":\"5\""), "{out}");
    }

    #[tokio::test]
    async fn runs_a_nested_leaf() {
        let out = builtin()
            .run(&argv("widgets parts list"))
            .await
            .expect("runs");
        assert!(out.contains("list_widget_parts"), "{out}");
    }

    #[tokio::test]
    async fn a_bare_switch_is_a_boolean_and_equals_form_works() {
        let out = builtin()
            .run(&argv("widgets list --archived --name=blue"))
            .await
            .expect("runs");
        assert!(out.contains("\"archived\":true"), "{out}");
        assert!(out.contains("\"name\":\"blue\""), "{out}");
    }

    #[tokio::test]
    async fn json_text_reaches_the_source_structured() {
        // Matches how the scripted host delivers aggregates, so a source sees
        // one shape regardless of which adapter it was called through.
        let args = vec![
            "widgets".to_string(),
            "create".to_string(),
            "--tags".to_string(),
            "[\"a\",\"b\"]".to_string(),
            "--name".to_string(),
            "w".to_string(),
        ];
        let out = builtin().run(&args).await.expect("runs");
        assert!(out.contains("\"tags\":[\"a\",\"b\"]"), "{out}");
    }

    #[tokio::test]
    async fn a_version_like_value_stays_a_string() {
        // Guessing types here would turn `1.20` into 1.2 before the source's
        // own schema ever sees it.
        let out = builtin()
            .run(&argv("widgets list --version 1.20"))
            .await
            .expect("runs");
        assert!(out.contains("\"version\":\"1.20\""), "{out}");
    }

    #[tokio::test]
    async fn help_beats_execution() {
        let out = builtin()
            .run(&argv("widgets create --help"))
            .await
            .expect("help");
        assert!(out.contains("everruns widgets create"), "{out}");
        assert!(
            !out.contains("\"ran\""),
            "help must not run the command: {out}"
        );
    }

    /// A Framework host whose domain has nothing to do with everruns.
    struct BrandedSource;

    #[async_trait]
    impl CliCommandSource for BrandedSource {
        fn root(&self) -> &str {
            "acme"
        }

        fn specs(&self) -> Vec<CliCommandSpec> {
            vec![CliCommandSpec {
                wire_name: "send_invoice".into(),
                description: "Send an invoice.".into(),
                route: CliRoute::new(&["invoices"], "send"),
            }]
        }

        async fn dispatch(&self, wire_name: &str, params: Value) -> Result<String, String> {
            Ok(serde_json::json!({ "ran": wire_name, "params": params }).to_string())
        }
    }

    fn branded() -> CliBuiltin {
        CliBuiltin::new(Arc::new(BrandedSource))
    }

    #[tokio::test]
    async fn a_host_names_its_own_root() {
        let out = branded()
            .run(&argv("invoices send --id 7"))
            .await
            .expect("runs under the host's own root");
        assert!(out.contains("\"ran\":\"send_invoice\""), "{out}");
    }

    #[tokio::test]
    async fn help_is_rendered_in_the_hosts_own_name() {
        let out = branded().run(&[]).await.expect("root help");
        assert!(out.contains("acme"), "{out}");
        assert!(
            !out.contains("everruns"),
            "a host's help must not wear another brand: {out}"
        );
    }

    #[tokio::test]
    async fn the_rewriter_follows_the_hosts_root() {
        let tree = CliTree::from_source(&BrandedSource);
        assert_eq!(rewrite("acme invoices send", &tree), "send_invoice");
        // The default token is just another word to a host that renamed it.
        assert_eq!(
            rewrite("everruns invoices send", &tree),
            "everruns invoices send"
        );
    }

    #[tokio::test]
    async fn the_default_root_is_unchanged() {
        let tree = CliTree::from_source(&TestSource);
        assert_eq!(tree.root(), "everruns");
        assert_eq!(rewrite("everruns widgets list", &tree), "list_widgets");
    }

    #[tokio::test]
    async fn an_unknown_noun_names_the_noun_not_the_last_word() {
        // A live model that guesses the wrong noun types a whole invocation,
        // not one word. Blaming the trailing token sends it looking for a verb
        // when the noun is what does not exist.
        let error = builtin()
            .run(&argv("gadgets resize thing 4"))
            .await
            .expect_err("an unknown noun must fail");
        assert!(
            error.contains("unknown command `gadgets`"),
            "should name the first unresolvable segment: {error}"
        );
    }

    #[tokio::test]
    async fn help_on_an_unknown_noun_is_an_error_not_root_help() {
        // Rendering root help here is worse than saying nothing: it is the
        // same shape as a valid `everruns widgets --help`, so the caller reads
        // a typo as a working command.
        let error = builtin()
            .run(&argv("gadgets --help"))
            .await
            .expect_err("help on an unknown noun must fail");
        assert!(
            error.contains("unknown command `gadgets`"),
            "should name the unknown noun: {error}"
        );
    }

    #[tokio::test]
    async fn help_on_a_real_node_still_succeeds() {
        let out = builtin().run(&argv("widgets --help")).await.expect("help");
        assert!(out.contains("list"), "{out}");
        assert!(out.contains("parts"), "{out}");
    }

    #[tokio::test]
    async fn root_help_lists_nouns() {
        let out = builtin().run(&[]).await.expect("root help");
        assert!(out.contains("widgets"), "{out}");
        assert!(out.contains("The widgets."), "{out}");
    }

    #[tokio::test]
    async fn an_unknown_verb_is_an_error_naming_real_neighbours() {
        let error = builtin()
            .run(&argv("widgets lst"))
            .await
            .expect_err("unknown verb fails");
        assert!(error.contains("unknown command `lst`"), "{error}");
        assert!(error.contains("create"), "{error}");
    }

    #[tokio::test]
    async fn a_source_error_reaches_the_caller() {
        let error = builtin()
            .run(&argv("widgets create"))
            .await
            .expect_err("source rejects");
        assert!(error.contains("missing --name"), "{error}");
    }

    #[tokio::test]
    async fn a_stray_positional_is_rejected_with_guidance() {
        let error = builtin()
            .run(&argv("widgets list oops"))
            .await
            .expect_err("bare value is not a flag");
        assert!(error.contains("--flag value"), "{error}");
    }
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
                route: CliRoute::new(&["widgets"], "list"),
            }]
        }
        async fn dispatch(&self, _wire: &str, _params: Value) -> Result<String, String> {
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
