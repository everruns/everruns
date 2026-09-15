// The `everruns` command tree, sourced from this server's domain-command
// catalog.
//
// Grammar, help rendering, and the statement-boundary rewrite are shared with
// every other host through `everruns_integrations_bashkit::cli`; this module
// only supplies the commands and their node descriptions. Keeping the source
// separate from the tree is what lets a Framework application expose the same
// spelling over entirely different commands.

use std::sync::OnceLock;

use async_trait::async_trait;

use crate::domains::common::CommandDescriptor;
use everruns_cli_contract::ContractCommand;
use everruns_integrations_bashkit::cli::{CliCommandSource, CliCommandSpec, CliTree};

pub use everruns_integrations_bashkit::cli::{HELP_BUILTIN, ROOT, render_help, rewrite};

/// One-line summaries for grouping nodes.
///
/// A node has no `CommandMeta` to borrow a description from, and the fallback
/// ("agents — agents commands") spends prompt budget teaching nothing.
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

/// Tree membership comes from `Command::cli()`, which defaults to `None`.
pub struct InventoryCommandSource;

#[async_trait]
impl CliCommandSource for InventoryCommandSource {
    fn specs(&self) -> Vec<CliCommandSpec> {
        contracts()
            .iter()
            .map(|contract| CliCommandSpec {
                wire_name: contract.wire_name.clone(),
                description: contract.description.clone(),
                path: contract.path.clone(),
                verb: contract.verb.clone(),
                command: contract.clap_command(&format!("{ROOT} {}", contract.spelling())),
            })
            .collect()
    }

    fn node_about(&self) -> Vec<(String, String)> {
        NODE_ABOUT
            .iter()
            .map(|(path, about)| ((*path).to_string(), (*about).to_string()))
            .collect()
    }

    async fn dispatch(
        &self,
        _wire_name: &str,
        _matches: clap::ArgMatches,
    ) -> Result<String, String> {
        // The scripted host dispatches through its own per-command builtins,
        // which already carry policy and error handling. This source exists to
        // describe the command line, not to re-enter dispatch.
        Err("dispatch is owned by the scripted toolset on this host".to_string())
    }
}

/// Every routed command's contract, built once from inventory.
///
/// One command declares its parameters as a Rust type and its presentation as
/// a `CliRoute`; this is where the two meet. `everruns-cli` mounts the same
/// values, so the flags a person types and the flags an agent types are the
/// same flags by construction rather than by review.
pub fn contracts() -> &'static [ContractCommand] {
    static CONTRACTS: OnceLock<Vec<ContractCommand>> = OnceLock::new();
    CONTRACTS.get_or_init(|| {
        let mut built: Vec<ContractCommand> = inventory::iter::<CommandDescriptor>
            .into_iter()
            .filter_map(|desc| {
                let route = (desc.cli)()?;
                let meta = (desc.meta)();
                Some(everruns_cli_contract::schema::contract_for(
                    meta.name,
                    meta.description,
                    meta.method,
                    meta.path,
                    &route,
                    &(desc.param_schema)(),
                ))
            })
            .collect();
        built.sort_by_key(|contract| contract.spelling());
        built
    })
}

/// One routed command's contract, by its wire name.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "`everruns-cli` mounts these \
    next; the lookup is here because it belongs beside `contracts`"
    )
)]
pub fn contract(wire_name: &str) -> Option<&'static ContractCommand> {
    contracts()
        .iter()
        .find(|contract| contract.wire_name == wire_name)
}

static TREE: OnceLock<CliTree> = OnceLock::new();

/// The process-wide tree, built once from inventory.
///
/// Feature gating is deliberately not applied: the tree is shared across orgs,
/// and a feature-disabled command still fails closed at dispatch through its
/// own policy and the catalog's exposure rules. Gating the spelling too would
/// make one script legal in one org and a parse error in another.
pub fn tree() -> &'static CliTree {
    TREE.get_or_init(|| CliTree::from_source(&InventoryCommandSource))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw(input: &str) -> String {
        rewrite(input, tree())
    }

    /// The checked-in contract artifact still matches inventory.
    ///
    /// `everruns-cli` reads the artifact, this crate owns the commands, and
    /// nothing links both. A guard is what keeps them the same thing: without
    /// it, a new command or a re-spelled flag reaches the agent-facing tree
    /// immediately and the CLI never hears about it.
    #[test]
    fn the_checked_in_contract_matches_inventory() {
        let generated =
            serde_json::to_string_pretty(contracts()).expect("contracts serialize") + "\n";
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../cli-contract/commands.json");

        if std::env::var("UPDATE_CLI_CONTRACT_COMMANDS").is_ok() {
            std::fs::write(path, &generated).expect("write commands.json");
            return;
        }

        let checked_in = std::fs::read_to_string(path).expect("read commands.json");
        assert_eq!(
            checked_in.trim(),
            generated.trim(),
            "crates/cli-contract/commands.json is stale. Run \
             `UPDATE_CLI_CONTRACT_COMMANDS=1 cargo test -p everruns-server \
             the_checked_in_contract_matches_inventory`, then check what moved: \
             everruns-cli mounts this file, so a change here changes what people type."
        );
    }

    /// Every routed command compiles into a parser, against the schemas the
    /// catalog really publishes rather than a fixture.
    ///
    /// clap panics on a malformed command (a duplicate argument id, a bad
    /// value-parser pairing), and it would panic inside the agent's shell.
    /// Building all of them here is what keeps a newly routed command from
    /// discovering that at runtime.
    #[test]
    fn every_routed_command_compiles_into_a_parser() {
        assert!(!contracts().is_empty(), "no commands declare a CLI route");
        for contract in contracts() {
            let help = contract
                .clap_command(&format!("{ROOT} {}", contract.spelling()))
                .render_long_help()
                .to_string();
            assert!(
                help.contains(&format!("Wire name: {}", contract.wire_name)),
                "{}: {help}",
                contract.spelling()
            );
        }
    }

    /// Yolop's bar, made structural: a command an agent cannot learn from
    /// `--help` is one it will guess at instead. An intent line without a
    /// command teaches nothing runnable, and a command line without an intent
    /// only restates syntax the caller could have guessed.
    #[test]
    fn every_routed_command_carries_a_worked_example() {
        let bare: Vec<&str> = contracts()
            .iter()
            .filter(|contract| {
                contract.examples.is_empty()
                    || contract
                        .examples
                        .iter()
                        .any(|example| example.intent.is_empty() || example.command.is_empty())
            })
            .map(|contract| contract.wire_name.as_str())
            .collect();
        assert!(
            bare.is_empty(),
            "commands without a worked example: {bare:?}"
        );
    }

    /// An example is the line a caller copies, so it has to parse. These used
    /// to be prose: twelve commands documented a bare-word form
    /// (`everruns sessions archive ses_01h9`) that no parser accepted, because
    /// the command never declared a positional.
    #[test]
    fn every_worked_example_parses_against_its_own_command() {
        for contract in contracts() {
            let display = format!("{ROOT} {}", contract.spelling());
            for example in &contract.examples {
                // Only the arguments: the spelling itself is the command name.
                let Some(rest) = example.command.strip_prefix(&display) else {
                    panic!(
                        "example for {} does not start with `{display}`: {}",
                        contract.wire_name, example.command
                    );
                };
                let argv = shell_words(rest);

                let parser = contract.clap_command(&display);
                let full = std::iter::once(display.clone()).chain(argv);
                if let Err(error) = parser.try_get_matches_from(full) {
                    panic!(
                        "example for {} does not parse:\n  {}\n{error}",
                        contract.wire_name, example.command
                    );
                }
            }
        }
    }

    /// Split an example's arguments the way a shell would, so a quoted value
    /// stays one argument. An example is written to be pasted into a shell, so
    /// `--skill-md "$(cat SKILL.md)"` is one argument, not two; splitting on
    /// whitespace would fail examples that are perfectly correct. Redirection
    /// and pipes past the command are the shell's business, not the parser's.
    fn shell_words(line: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut quote: Option<char> = None;
        let mut started = false;

        for ch in line.chars() {
            match (quote, ch) {
                (Some(open), _) if ch == open => quote = None,
                (Some(_), _) => current.push(ch),
                (None, '\'' | '"') => {
                    quote = Some(ch);
                    started = true;
                }
                (None, c) if c.is_whitespace() => {
                    if started {
                        words.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                (None, '>' | '|' | '&') => break,
                (None, c) => {
                    current.push(c);
                    started = true;
                }
            }
        }
        if started {
            words.push(current);
        }
        words
    }

    /// The flat command surface rewrites a bare word into `--<field>` for
    /// commands declaring `positional_arg`. The contract declares the same
    /// thing as `.at(1)`. Two declarations of one fact drift, so assert they
    /// agree rather than hoping.
    #[test]
    fn declared_positionals_agree_with_the_flat_surface() {
        for desc in inventory::iter::<CommandDescriptor> {
            let Some(route) = (desc.cli)() else { continue };
            let meta = (desc.meta)();
            let flat = (desc.positional_arg)();
            let declared = route.args.iter().find(|arg| arg.position.is_some());

            match (flat, declared) {
                (Some(field), Some(arg)) => assert_eq!(
                    field, arg.field,
                    "{}: flat surface takes `{field}` positionally, the contract takes `{}`",
                    meta.name, arg.field
                ),
                (None, Some(_)) | (Some(_), None) => {}
                (None, None) => {}
            }
        }
    }

    /// Pagination reaches commands through `#[serde(flatten)]`, which renders
    /// as an `allOf` branch in the schema. A parser that only read top-level
    /// properties would call `--limit` an unknown flag.
    #[test]
    fn a_flattened_field_is_a_real_flag() {
        let contract = contract("list_agents").expect("agents list is routed");
        let parser = contract.clap_command("everruns agents list");
        let matches = parser
            .try_get_matches_from(["everruns agents list", "--limit", "10"])
            .expect("--limit parses");
        let params = everruns_cli_contract::params_from(contract, &matches);
        assert_eq!(params["limit"], 10);
    }

    /// The presentation the shipped CLI chose reaches the agent-facing tree.
    #[test]
    fn a_short_option_from_the_cli_works_here_too() {
        let contract = contract("create_agent").expect("agents create is routed");
        let parser = contract.clap_command("everruns agents create");
        let matches = parser
            .try_get_matches_from([
                "everruns agents create",
                "--name",
                "triage",
                "--system-prompt",
                "Triage incoming issues",
                "-H",
                "generic",
            ])
            .expect("-H parses");
        let params = everruns_cli_contract::params_from(contract, &matches);
        assert_eq!(params["harness_name"], "generic");
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
        let text = render_help(tree(), "", None).expect("root help");
        assert_eq!(
            listed_commands(&text),
            vec!["agents", "mcp-servers", "sessions", "skills"],
            "{text}"
        );
    }

    #[test]
    fn node_help_lists_direct_children_only() {
        let listed = listed_commands(&render_help(tree(), "agents", None).unwrap());
        assert!(listed.contains(&"list".to_string()), "{listed:?}");
        assert!(listed.contains(&"versions".to_string()), "{listed:?}");
        // A grandchild verb belongs to `agents versions`, not to `agents`.
        assert!(!listed.contains(&"set-default".to_string()), "{listed:?}");
    }

    #[test]
    fn leaf_help_carries_flags_examples_and_the_wire_name() {
        let text = render_help(tree(), "agents list", None).expect("leaf help");
        // The usage line reads back what the caller typed, not the alias.
        assert!(text.contains("Usage: everruns agents list"), "{text}");
        assert!(text.contains("Examples:"), "{text}");
        assert!(text.contains("everruns agents list --search"), "{text}");
        assert!(text.contains("Wire name: list_agents"), "{text}");
    }

    #[test]
    fn unknown_verb_help_is_an_error_naming_real_neighbours() {
        let error = render_help(tree(), "agents", Some("lst")).expect_err("should be an error");
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

    /// The leaf help has to show real, typeable flags: the whole reason a tree
    /// can afford `--help` is that each leaf is small. If this ever renders an
    /// empty flag list the surface silently regresses to "guess the schema".
    ///
    /// It is the contract's parser that renders this, on a host that cannot
    /// reach clap to parse. Help needs no argv, so the words describing a
    /// command are the same words `everruns-cli` uses for it.
    #[test]
    fn leaf_usage_renders_real_flags_under_the_tree_spelling() {
        let text = render_help(tree(), "mcp-servers create", None).expect("leaf help");

        assert!(text.contains("everruns mcp-servers create"), "{text}");
        assert!(text.contains("--name"), "{text}");
        assert!(text.contains("--url"), "{text}");
        assert!(text.contains("Wire name: create_mcp_server"), "{text}");
        // The worked example is part of what makes a leaf learnable.
        assert!(text.contains("Register an MCP server"), "{text}");
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use crate::domains::common::catalog_entries_with_schemas;
    use everruns_platform::FeatureFlags;

    /// A CLI does not advertise itself the way a tool schema does, so
    /// discovery has to carry the spelling. If this regresses, the tree still
    /// works but nothing tells a model it exists.
    #[test]
    fn discovery_carries_the_tree_spelling_for_declared_commands() {
        let flags = FeatureFlags::default();
        let entries = catalog_entries_with_schemas(false, &flags);

        let agents_list = entries
            .iter()
            .find(|entry| entry.name == "list_agents")
            .expect("list_agents is registered");
        assert_eq!(agents_list.cli.as_deref(), Some("agents list"));

        // Opt-in: a command that never declared a route stays absent from the
        // tree rather than being derived into it.
        let undeclared = entries
            .iter()
            .find(|entry| entry.cli.is_none())
            .expect("the tranche is a subset, not the whole catalog");
        assert!(tree().leaf(undeclared.name).is_none());
    }
}

/// End-to-end coverage for the pieces unit tests cannot reach: that the text
/// the rewriter emits is valid bash, that `everruns_help` is actually
/// registered and its flags parse, and that a caller gets help rather than an
/// interpreter error.
///
/// These drive the real Bashkit interpreter. They deliberately do not need a
/// database: help is the one branch of the surface that touches no domain
/// command, so it can be proven here rather than only in the live eval.
#[cfg(test)]
mod bashkit_tests {
    use super::*;
    use crate::api::mcp_endpoint::catalog::{help_tool_def, make_help_callback};
    use bashkit::{ScriptedTool, Tool, ToolRequest};

    async fn run(script: &str) -> (i32, String, String) {
        let tool = ScriptedTool::builder("everruns")
            .async_tool_fn(help_tool_def(), make_help_callback())
            .sanitize_errors(false)
            .build();
        let rewritten = rewrite(script, tree());
        let response = Tool::execute(&tool, ToolRequest::new(rewritten)).await;
        (response.exit_code, response.stdout, response.stderr)
    }

    #[tokio::test]
    async fn root_help_runs_through_the_interpreter() {
        let (code, stdout, stderr) = run("everruns --help").await;
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(stdout.contains("agents"), "{stdout}");
        assert!(stdout.contains("mcp-servers"), "{stdout}");
    }

    #[tokio::test]
    async fn node_help_runs_and_lists_verbs() {
        let (code, stdout, stderr) = run("everruns agents --help").await;
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(stdout.contains("versions"), "{stdout}");
        assert!(stdout.contains("check-name"), "{stdout}");
    }

    #[tokio::test]
    async fn leaf_help_runs_and_shows_real_flags() {
        let (code, stdout, stderr) = run("everruns agents list --help").await;
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(stdout.contains("everruns agents list"), "{stdout}");
        assert!(stdout.contains("--search"), "{stdout}");
        assert!(stdout.contains("Wire name: list_agents"), "{stdout}");
    }

    #[tokio::test]
    async fn a_bare_noun_shows_its_children_instead_of_failing() {
        let (code, stdout, stderr) = run("everruns agents").await;
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(stdout.contains("Usage: everruns agents"), "{stdout}");
    }

    #[tokio::test]
    async fn an_unknown_verb_exits_nonzero_and_names_real_neighbours() {
        let (code, stdout, stderr) = run("everruns agents lst").await;
        assert_ne!(code, 0, "an unusable command must not look like success");
        let text = format!("{stdout}{stderr}");
        assert!(text.contains("unknown command `lst`"), "{text}");
        assert!(text.contains("list"), "{text}");
    }

    #[tokio::test]
    async fn help_output_survives_a_pipeline() {
        // The rewrite has to emit something the interpreter can still compose.
        let (code, stdout, stderr) = run("everruns --help | head -3").await;
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(stdout.lines().count() <= 3, "{stdout}");
    }
}

/// The tree must not become a route around the read-only toolset.
#[cfg(test)]
mod read_only_tests {
    use super::*;
    use crate::api::mcp_endpoint::catalog::ToolsetMode;
    use crate::domains::common::CommandDescriptor;

    /// THREAT[TM-MCP-002]: `query` exposes read-only commands only. The rewrite
    /// maps a tree spelling onto a wire name, and a mutating wire name is
    /// simply not registered as a builtin in read-only mode, so the rewritten
    /// script fails with command-not-found rather than mutating. This asserts
    /// the property the rewrite depends on: it can only ever emit a name that
    /// exists in the catalog, so mode gating stays the single decision point.
    #[test]
    fn every_tree_leaf_maps_to_a_registered_command_with_honest_read_only_status() {
        let mutating: Vec<&str> = inventory::iter::<CommandDescriptor>
            .into_iter()
            .filter_map(|desc| {
                let route = (desc.cli)()?;
                let meta = (desc.meta)();
                // The rewrite target must be the command's own wire name...
                let leaf = tree().leaf(&route.spelling()).expect("declared leaf");
                assert_eq!(leaf.command, meta.name);
                // ...and its read-only classification is the command's, not
                // something the tree can restate or soften.
                (!(desc.read_only)()).then_some(meta.name)
            })
            .collect();

        assert!(
            mutating.contains(&"create_agent"),
            "the tranche should include mutating commands, or this proves nothing"
        );
        assert_ne!(ToolsetMode::ReadOnly, ToolsetMode::Full);
    }
}
