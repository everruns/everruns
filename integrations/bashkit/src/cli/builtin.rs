//! The bash-tool adapter: a builtin that receives raw argv and walks the tree.

use super::*;

// ============================================================================
// Bash-tool adapter
// ============================================================================

/// The `everruns` builtin for a plain bash tool.
///
/// Unlike a `ScriptedTool` host, a builtin here receives raw argv. That is the
/// whole difference, and it is what lets this path parse properly: the tree is
/// walked directly to resolve a leaf, and everything after the leaf goes to
/// the leaf's own [`clap::Command`], compiled from the schema the command
/// already publishes. See [`args`] for what that buys over hand-parsing.
///
/// Help splits along the same line. Nodes and the root are the tree's to
/// render, because their content *is* the tree. A leaf's help is clap's,
/// generated from the same schema as the parse, so what a caller reads and
/// what they are then held to are one artifact.
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

        // A resolved leaf handles its own `--help`, so the flag rides along
        // with the rest of argv rather than being intercepted here. Help still
        // beats execution: clap renders it and parses nothing.
        if let Some(leaf) = self.tree.leaf(&spelling) {
            return CliPlan::Run {
                spelling,
                wire_name: leaf.command.clone(),
                args: args[rest.min(args.len())..].to_vec(),
            };
        }
        if wants_help {
            // No leaf: the caller is asking what exists under a node, which is
            // the tree's question to answer. It must still fail when the path
            // is not real, or a typo renders as a working help page.
            let (path, unknown) = split_at_unknown(&self.tree, &spelling);
            return CliPlan::Help { path, unknown };
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
            CliPlan::Help { path, unknown } => render_help(&self.tree, &path, unknown.as_deref()),
            CliPlan::Run {
                spelling,
                wire_name,
                args,
            } => {
                let leaf = self
                    .tree
                    .leaf(&spelling)
                    .ok_or_else(|| format!("unknown command `{spelling}`"))?;
                let parser = leaf
                    .parser
                    .clone()
                    .name(format!("{} {spelling}", self.tree.root()));
                let argv =
                    std::iter::once(parser.get_name().to_string()).chain(args.iter().cloned());

                match parser.clone().try_get_matches_from(argv) {
                    Ok(matches) => self.source.dispatch(&wire_name, matches).await,
                    // `--help` is a clap response, not a failure: it printed
                    // what the caller asked for and ran nothing.
                    Err(error) if is_display(&error) => Ok(error.render().to_string()),
                    Err(error) => Err(error.render().to_string()),
                }
            }
        }
    }
}

/// Whether a clap error is a rendered response rather than a rejection.
fn is_display(error: &clap::Error) -> bool {
    use clap::error::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    )
}

enum CliPlan {
    Help {
        path: String,
        unknown: Option<String>,
    },
    Run {
        /// Tree spelling the caller typed, for usage and errors.
        spelling: String,
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
pub(super) fn split_at_unknown(tree: &CliTree, path: &str) -> (String, Option<String>) {
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
pub(super) fn interned_hint(root: &str) -> &'static str {
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

pub(super) fn ensure_trailing_newline(mut text: String) -> String {
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
    ///
    /// It builds its own `clap::Command`s, which is the point of the seam:
    /// this crate never learns what a widget is, and a host is free to spell
    /// its commands however it likes.
    struct TestSource;

    fn widgets_list() -> clap::Command {
        clap::Command::new("list")
            .about("List widgets.")
            .color(clap::ColorChoice::Never)
            .arg(
                clap::Arg::new("limit")
                    .long("limit")
                    .value_parser(clap::value_parser!(i64))
                    .help("Maximum rows."),
            )
            .arg(
                clap::Arg::new("include_archived")
                    .long("include_archived")
                    .num_args(0..=1)
                    .default_missing_value("true")
                    .value_parser(clap::builder::BoolishValueParser::new()),
            )
            .arg(clap::Arg::new("status").long("status").value_parser(
                clap::builder::PossibleValuesParser::new(["active", "retired"]),
            ))
            .arg(
                clap::Arg::new("tag")
                    .long("tag")
                    .action(clap::ArgAction::Append)
                    .value_delimiter(','),
            )
            .arg(
                clap::Arg::new("agent_id")
                    .long("agent-id")
                    .alias("agent_id"),
            )
    }

    fn widgets_create() -> clap::Command {
        clap::Command::new("create")
            .about("Create a widget.")
            .color(clap::ColorChoice::Never)
            .arg(clap::Arg::new("name").long("name").required(true))
    }

    fn widget_parts_list() -> clap::Command {
        clap::Command::new("list")
            .about("List the parts of a widget.")
            .color(clap::ColorChoice::Never)
            .arg(clap::Arg::new("id").long("id"))
            // The bare-word spelling, as a real positional.
            .arg(
                clap::Arg::new("id_positional")
                    .index(1)
                    .conflicts_with("id"),
            )
    }

    /// Render what clap parsed, for assertions. A real source reads the fields
    /// it declared; this one only needs to show what arrived.
    fn seen(matches: &clap::ArgMatches) -> Value {
        let mut object = serde_json::Map::new();
        for id in matches.ids() {
            let key = id.as_str();
            let field = key.strip_suffix("_positional").unwrap_or(key);
            if let Ok(Some(value)) = matches.try_get_one::<bool>(key) {
                object.insert(field.into(), Value::Bool(*value));
                continue;
            }
            if let Ok(Some(value)) = matches.try_get_one::<i64>(key) {
                object.insert(field.into(), Value::Number((*value).into()));
                continue;
            }
            if let Ok(Some(values)) = matches.try_get_many::<String>(key) {
                let values: Vec<&String> = values.collect();
                if values.len() > 1 {
                    object.insert(
                        field.into(),
                        Value::Array(values.into_iter().cloned().map(Value::String).collect()),
                    );
                } else if let Some(value) = values.first() {
                    object.insert(field.into(), Value::String((*value).clone()));
                }
            }
        }
        Value::Object(object)
    }

    #[async_trait]
    impl CliCommandSource for TestSource {
        fn specs(&self) -> Vec<CliCommandSpec> {
            vec![
                CliCommandSpec {
                    wire_name: "list_widgets".into(),
                    description: "List widgets.".into(),
                    path: vec!["widgets".into()],
                    verb: "list".into(),
                    command: widgets_list().after_help(
                        "Examples:\n  List the five most recent widgets:\n    \
                         everruns widgets list --limit 5\n\nWire name: list_widgets",
                    ),
                },
                CliCommandSpec {
                    wire_name: "create_widget".into(),
                    description: "Create a widget.".into(),
                    path: vec!["widgets".into()],
                    verb: "create".into(),
                    command: widgets_create(),
                },
                CliCommandSpec {
                    wire_name: "list_widget_parts".into(),
                    description: "List the parts of a widget.".into(),
                    path: vec!["widgets".into(), "parts".into()],
                    verb: "list".into(),
                    command: widget_parts_list(),
                },
            ]
        }

        fn node_about(&self) -> Vec<(String, String)> {
            vec![("widgets".to_string(), "The widgets.".to_string())]
        }

        async fn dispatch(
            &self,
            wire_name: &str,
            matches: clap::ArgMatches,
        ) -> Result<String, String> {
            let params = seen(&matches);
            // A name clap cannot know is taken: the errors a source still owns
            // are the ones about its own state, not about argument shape.
            if wire_name == "create_widget" && params.get("name") == Some(&Value::from("taken")) {
                return Err("create_widget: `taken` already exists".to_string());
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
        // A number, not the string "5": the schema said integer, so the source
        // is handed the type it declared instead of text to coerce.
        assert!(out.contains("\"limit\":5"), "{out}");
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
            .run(&argv("widgets list --include_archived --status=active"))
            .await
            .expect("runs");
        assert!(out.contains("\"include_archived\":true"), "{out}");
        assert!(out.contains("\"status\":\"active\""), "{out}");
    }

    /// Both spellings of a switch work. Models write either, and one of them
    /// erroring is a round trip spent on syntax rather than on the task.
    #[tokio::test]
    async fn a_switch_also_accepts_an_explicit_value() {
        let out = builtin()
            .run(&argv("widgets list --include_archived false"))
            .await
            .expect("runs");
        assert!(out.contains("\"include_archived\":false"), "{out}");
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
                path: vec!["invoices".into()],
                verb: "send".into(),
                command: clap::Command::new("send")
                    .about("Send an invoice.")
                    .color(clap::ColorChoice::Never)
                    .arg(clap::Arg::new("to").long("to")),
            }]
        }

        async fn dispatch(
            &self,
            wire_name: &str,
            matches: clap::ArgMatches,
        ) -> Result<String, String> {
            Ok(serde_json::json!({ "ran": wire_name, "params": seen(&matches) }).to_string())
        }
    }

    fn branded() -> CliBuiltin {
        CliBuiltin::new(Arc::new(BrandedSource))
    }

    #[tokio::test]
    async fn a_host_names_its_own_root() {
        let out = branded()
            .run(&argv("invoices send --to acme"))
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
            .run(&argv("widgets create --name taken"))
            .await
            .expect_err("source rejects");
        assert!(error.contains("already exists"), "{error}");
    }

    #[tokio::test]
    async fn a_stray_positional_is_rejected_with_guidance() {
        let error = builtin()
            .run(&argv("widgets list oops"))
            .await
            .expect_err("bare value is not a flag");
        assert!(error.contains("unexpected argument 'oops'"), "{error}");
        // The usage block rides along, so the correction is in the same
        // response as the complaint.
        assert!(error.contains("Usage: everruns widgets list"), "{error}");
    }

    // ========================================================================
    // What parsing with clap buys
    //
    // Each of these is a shape the hand-rolled parser accepted or mangled.
    // ========================================================================

    /// The headline fix. The old parser kept an unrecognized flag as a string
    /// property and passed it on, so a typo became a silently dropped argument
    /// or an error from somewhere with no idea what the caller typed.
    #[tokio::test]
    async fn an_unknown_flag_is_rejected_where_the_caller_can_still_fix_it() {
        let error = builtin()
            .run(&argv("widgets list --limti 10"))
            .await
            .expect_err("a misspelled flag is not a parameter");
        assert!(error.contains("--limti"), "{error}");
        // clap knows the real flags, so it can name the one that was meant.
        assert!(
            error.contains("--limit"),
            "did not suggest --limit: {error}"
        );
    }

    /// A required field is enforced before dispatch, with usage attached,
    /// rather than surfacing as a deserialization error from the far side.
    #[tokio::test]
    async fn a_required_field_is_enforced_before_dispatch() {
        let error = builtin()
            .run(&argv("widgets create"))
            .await
            .expect_err("--name is required");
        assert!(error.contains("--name"), "{error}");
        assert!(!error.contains("\"ran\""), "must not dispatch: {error}");
    }

    /// Schemas name fields in snake_case because they are generated from Rust
    /// structs. A CLI caller reasonably types kebab, and `crates/cli` spells
    /// it that way, so both reach the same parameter.
    #[tokio::test]
    async fn a_flag_answers_to_both_snake_case_and_kebab_case() {
        for line in ["widgets list --agent_id a_1", "widgets list --agent-id a_1"] {
            let out = builtin().run(&argv(line)).await.expect("runs");
            assert!(out.contains("\"agent_id\":\"a_1\""), "{line}: {out}");
        }
    }

    /// A nominated positional is a real clap positional, so the bare-word form
    /// parses here rather than being faked by rewriting the command string
    /// before the interpreter sees it.
    #[tokio::test]
    async fn a_nominated_field_also_takes_a_bare_word() {
        let out = builtin()
            .run(&argv("widgets parts list w_1"))
            .await
            .expect("runs");
        assert!(out.contains("\"id\":\"w_1\""), "{out}");

        let flagged = builtin()
            .run(&argv("widgets parts list --id w_1"))
            .await
            .expect("the flag spelling still works");
        assert!(flagged.contains("\"id\":\"w_1\""), "{flagged}");
    }

    /// Giving both spellings is ambiguous, so it is an error rather than a
    /// silent winner.
    #[tokio::test]
    async fn the_two_spellings_of_one_field_conflict() {
        let error = builtin()
            .run(&argv("widgets parts list w_1 --id w_2"))
            .await
            .expect_err("one field, one value");
        assert!(error.contains("cannot be used with"), "{error}");
    }

    /// A declared `enum` reaches clap, so a wrong value is corrected against
    /// the real options instead of dispatched and rejected downstream.
    #[tokio::test]
    async fn a_declared_enum_lists_its_options_on_a_wrong_value() {
        let error = builtin()
            .run(&argv("widgets list --status bogus"))
            .await
            .expect_err("not a declared status");
        assert!(error.contains("active"), "{error}");
        assert!(error.contains("retired"), "{error}");
    }

    /// An array field repeats and comma-splits, and arrives as a JSON array.
    #[tokio::test]
    async fn an_array_field_repeats_and_comma_splits() {
        let out = builtin()
            .run(&argv("widgets list --tag a,b --tag c"))
            .await
            .expect("runs");
        assert!(out.contains("\"tag\":[\"a\",\"b\",\"c\"]"), "{out}");
    }

    /// A typed field rejects a value of the wrong type here, where the usage
    /// block is, rather than after a dispatch.
    #[tokio::test]
    async fn an_integer_field_rejects_a_non_number() {
        let error = builtin()
            .run(&argv("widgets list --limit soon"))
            .await
            .expect_err("not a number");
        assert!(error.contains("soon"), "{error}");
    }

    /// A leaf's help is clap's, generated from the same schema as the parse,
    /// so the flags a caller reads are exactly the ones they are held to.
    #[tokio::test]
    async fn leaf_help_comes_from_the_schema_and_names_the_wire_command() {
        let out = builtin()
            .run(&argv("widgets list --help"))
            .await
            .expect("help");
        assert!(out.contains("Usage: everruns widgets list"), "{out}");
        assert!(out.contains("--limit"), "{out}");
        assert!(out.contains("Maximum rows."), "{out}");
        // The flat name still works, and help is where a caller learns it.
        assert!(out.contains("Wire name: list_widgets"), "{out}");
        // Declared examples ride along with the leaf that has them.
        assert!(!out.contains("\"ran\""), "help must not run it: {out}");
    }

    /// The workspace links clap with its default features for `crates/cli`,
    /// and cargo unifies that across the build, so colour is on unless a
    /// command says otherwise. Escape bytes in a tool result are noise the
    /// model pays for and reads past.
    #[tokio::test]
    async fn output_carries_no_terminal_escapes() {
        let error = builtin()
            .run(&argv("widgets list --limti 10"))
            .await
            .expect_err("a misspelled flag");
        assert!(
            !error.contains('\u{1b}'),
            "escape bytes in output: {error:?}"
        );

        let help = builtin()
            .run(&argv("widgets list --help"))
            .await
            .expect("help");
        assert!(!help.contains('\u{1b}'), "escape bytes in help: {help:?}");
    }

    /// Defaults belong to the command, not to the parser in front of it.
    /// Sending clap's view of an untouched flag would overwrite a server-side
    /// default with a guess made here.
    #[tokio::test]
    async fn an_untouched_flag_is_not_sent() {
        let out = builtin().run(&argv("widgets list")).await.expect("runs");
        assert!(out.contains("\"params\":{}"), "{out}");
    }
}
