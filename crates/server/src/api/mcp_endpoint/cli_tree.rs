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
use serde_json::Value;

use crate::domains::common::CommandDescriptor;
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
        inventory::iter::<CommandDescriptor>
            .into_iter()
            .filter_map(|desc| {
                let route = (desc.cli)()?;
                let meta = (desc.meta)();
                Some(CliCommandSpec {
                    wire_name: meta.name.to_string(),
                    description: meta.description.to_string(),
                    route,
                })
            })
            .collect()
    }

    fn node_about(&self) -> Vec<(String, String)> {
        NODE_ABOUT
            .iter()
            .map(|(path, about)| ((*path).to_string(), (*about).to_string()))
            .collect()
    }

    async fn dispatch(&self, _wire_name: &str, _params: Value) -> Result<String, String> {
        // The scripted host dispatches through its own per-command builtins,
        // which already carry schema coercion, policy, and error handling.
        // This source exists to describe the tree, not to re-enter dispatch.
        Err("dispatch is owned by the scripted toolset on this host".to_string())
    }
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
