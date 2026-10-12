// The `everruns` command tree, sourced from this server's domain-command
// catalog.
//
// Resolution and help are the shared mapper's (`everruns_cli_contract::Mapper`),
// so a line resolves here exactly as it does in the terminal CLI and the
// worker's shell; this module only supplies the commands. The scripted
// toolset's `everruns` builtin (`command_line.rs`) runs what it resolves.

use std::sync::OnceLock;

use crate::domains::common::CommandDescriptor;
#[cfg(test)]
use everruns_cli_contract::CommandTree;
use everruns_cli_contract::mapper::NODE_ABOUT;
use everruns_cli_contract::{ContractCommand, Mapper};

pub use everruns_cli_contract::ROOT;

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
                let meta = (desc.meta)();
                // Internal worker plumbing is nobody's command line.
                if meta.is_internal() {
                    return None;
                }
                // Every command is part of the command line. A declared route
                // wins; the rest derive one from the REST path and the flat
                // name, so `--help` reaches the whole catalog rather than the
                // curated slice of it.
                everruns_cli_contract::schema::contract_with(
                    meta.name,
                    meta.description,
                    meta.method,
                    meta.path,
                    (desc.cli)().as_ref(),
                    &(desc.param_schema)(),
                )
            })
            .collect();
        built.sort_by_key(|contract| contract.spelling());
        built
    })
}

/// The contract one caller can use: commands the surfaces expose and whose
/// feature is enabled for them. `GET /v1/commands` serves this, so a client
/// learns the grammar from the server it is talking to.
pub fn contracts_for(
    feature_flags: &crate::records::FeatureFlags,
) -> Vec<&'static ContractCommand> {
    let enabled: std::collections::BTreeSet<&str> = inventory::iter::<CommandDescriptor>
        .into_iter()
        .map(|desc| (desc.meta)())
        .filter(|meta| super::catalog::is_exposed(meta) && meta.is_enabled(feature_flags))
        .map(|meta| meta.name)
        .collect();
    contracts()
        .iter()
        .filter(|contract| enabled.contains(contract.wire_name.as_str()))
        .collect()
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

/// The process-wide mapper over the live contract, built once from inventory.
///
/// Feature gating is deliberately not applied: the tree is shared across orgs,
/// and a feature-disabled command still fails closed at dispatch through its
/// own policy and the catalog's exposure rules. Gating the spelling too would
/// make one script legal in one org and a parse error in another.
pub fn mapper() -> &'static Mapper {
    static MAPPER: OnceLock<Mapper> = OnceLock::new();
    MAPPER.get_or_init(|| Mapper::new(ROOT, NODE_ABOUT, contracts()))
}

#[cfg(test)]
pub fn tree() -> &'static CommandTree {
    mapper().tree()
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_cli_contract::Resolution;

    /// The command catalog the offline eval subject grades against.
    ///
    /// `evals/platform-capability` runs its cases without a server, against an
    /// in-process fake. A fake built from a hand-curated command list would
    /// measure the model against a surface that does not exist; built from
    /// this, it measures it against the one that does. Descriptions and schemas
    /// are what `discover` returns, so the text the model reads while choosing
    /// a command is the real text.
    #[test]
    fn the_eval_catalog_matches_inventory() {
        // Built from inventory directly rather than through the feature-flag
        // filter: the artifact is the whole surface, not one deployment's
        // slice, so a case cannot pass or fail depending on which flags were
        // on when it was generated.
        let entries: Vec<serde_json::Value> = {
            let mut entries: Vec<serde_json::Value> = inventory::iter::<CommandDescriptor>
                .into_iter()
                // `discover` never shows internal worker commands.
                .filter(|desc| !(desc.meta)().is_internal())
                .map(|desc| {
                    let meta = (desc.meta)();
                    serde_json::json!({
                        "name": meta.name,
                        "category": meta.category,
                        "description": meta.description,
                        "method": meta.method,
                        "path": meta.path,
                        "read_only": (desc.read_only)(),
                        "positional_arg": (desc.positional_arg)(),
                        "cli": (desc.cli)().map(|route| route.spelling()),
                        "input_schema": (desc.param_schema)(),
                        "output_shape": (desc.output_shape)(),
                    })
                })
                .collect();
            entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            entries
        };
        let generated = serde_json::to_string_pretty(&entries).expect("catalog serializes") + "\n";
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../evals/platform-capability/catalog.json"
        );

        if std::env::var("UPDATE_EVAL_CATALOG").is_ok() {
            std::fs::write(path, &generated).expect("write catalog.json");
            return;
        }

        let checked_in = std::fs::read_to_string(path).expect("read catalog.json");
        assert_eq!(
            checked_in.trim(),
            generated.trim(),
            "evals/platform-capability/catalog.json is stale. Run \
             `UPDATE_EVAL_CATALOG=1 cargo test -p everruns-server \
             the_eval_catalog_matches_inventory`."
        );
    }

    /// No command presents its request body as a single `req` argument.
    ///
    /// A command struct that wraps its body in `req: SomeRequest` without
    /// `#[serde(flatten)]` deserializes from `{"req": {…}}`, which reaches a
    /// caller as `--req '{"summary":"…"}'`: a JSON blob where flags belong,
    /// undiscoverable from `--help` and unparseable by anything that does not
    /// already know the inner type. The HTTP handlers build these commands
    /// field-wise from a path param and a typed body, so flattening changes the
    /// command surface only and leaves the REST body untouched.
    ///
    /// 32 of 36 such commands already flattened; this is what keeps the other
    /// four from coming back.
    #[test]
    fn no_command_takes_its_request_body_as_one_argument() {
        let offenders: Vec<&str> = inventory::iter::<CommandDescriptor>
            .into_iter()
            .filter(|desc| {
                (desc.param_schema)()
                    .get("properties")
                    .and_then(|properties| properties.get("req"))
                    .is_some()
            })
            .map(|desc| (desc.meta)().name)
            .collect();

        assert!(
            offenders.is_empty(),
            "these commands expose a nested `req` object instead of flags; add \
             #[serde(flatten)] to the field: {offenders:?}"
        );
    }

    /// The node help the eval's shell serves, rendered by the shipped tree.
    ///
    /// The shell's discovery story is `--help`, so an eval whose help is
    /// hand-rolled measures the hand-rolled help. A leaf's help already comes
    /// from the contract's own `clap::Command` on both sides; only the root and
    /// the grouping nodes are the tree's to render, so those are what travels.
    #[test]
    fn the_eval_help_matches_the_shipped_tree() {
        let tree = tree();

        // Every node path in the tree, root first.
        let mut paths: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::from([String::new()]);
        for contract in contracts() {
            let words = &contract.path;
            for take in 1..=words.len() {
                paths.insert(words[..take].join(" "));
            }
        }

        let help: serde_json::Map<String, serde_json::Value> = paths
            .into_iter()
            .filter(|path| path.is_empty() || tree.is_node(path))
            .filter_map(|path| {
                tree.render_help(&path, None)
                    .ok()
                    .map(|text| (path, serde_json::Value::String(text)))
            })
            .collect();

        let generated = serde_json::to_string_pretty(&serde_json::Value::Object(help))
            .expect("help serializes")
            + "\n";
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../evals/platform-capability/help.json"
        );

        if std::env::var("UPDATE_EVAL_CATALOG").is_ok() {
            std::fs::write(path, &generated).expect("write help.json");
            return;
        }

        let checked_in = std::fs::read_to_string(path).expect("read help.json");
        assert_eq!(
            checked_in.trim(),
            generated.trim(),
            "evals/platform-capability/help.json is stale. Run \
             `UPDATE_EVAL_CATALOG=1 cargo test -p everruns-server the_eval_`."
        );
    }

    /// The canonical harness the offline eval subject reproduces: its system prompt
    /// and its one tool.
    ///
    /// The harness contract is that the platform is a command in the session's shell, so
    /// what the eval must reproduce is a `bash` tool and nothing else. Reading
    /// the schema off `BashTool` rather than restating it is what keeps the two
    /// arms of the A/B differing only in the surface under test.
    #[test]
    fn the_eval_harness_matches_the_shipped_one() {
        use everruns_core::Tool;
        use everruns_integrations::bashkit::BashTool;

        let bash = BashTool::default();
        let harness = serde_json::json!({
            "system_prompt": crate::platform_chat_agent::system_prompt(),
            "tools": [
                {
                    "name": bash.name(),
                    "description": bash.description(),
                    "schema": bash.parameters_schema(),
                },
            ]
        });
        let generated = serde_json::to_string_pretty(&harness).expect("harness serializes") + "\n";
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../evals/platform-capability/harness.json"
        );

        if std::env::var("UPDATE_EVAL_CATALOG").is_ok() {
            std::fs::write(path, &generated).expect("write harness.json");
            return;
        }

        let checked_in = std::fs::read_to_string(path).expect("read harness.json");
        assert_eq!(
            checked_in.trim(),
            generated.trim(),
            "evals/platform-capability/harness.json is stale. Run \
             `UPDATE_EVAL_CATALOG=1 cargo test -p everruns-server the_eval_`."
        );
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

    /// No command's spelling is also a group of other commands.
    ///
    /// clap cannot have `harnesses delete` be both a command and the parent of
    /// `harnesses delete destroy`: the group wins, and the leaf silently
    /// disappears from every surface that mounts the tree. Derived routes hit
    /// this whenever a REST action path extends a resource path, so the guard
    /// is structural rather than per command.
    #[test]
    fn no_command_is_also_a_group() {
        let spellings: std::collections::BTreeSet<String> =
            contracts().iter().map(|c| c.spelling()).collect();
        let shadowed: Vec<String> = contracts()
            .iter()
            .filter_map(|contract| {
                (1..contract.path.len() + 1).find_map(|depth| {
                    let prefix = contract.path[..depth].join(" ");
                    spellings
                        .contains(&prefix)
                        .then(|| format!("`{prefix}` is shadowed by `{}`", contract.spelling()))
                })
            })
            .collect();
        assert!(shadowed.is_empty(), "{shadowed:#?}");
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

    /// Yolop's bar, made structural: every command carries a worked example.
    ///
    /// `--help` is the only way an agent learns this surface, since no model
    /// has it in its training data. An agent reading help is choosing between
    /// commands, and the example's intent line is what tells it when to reach
    /// for one. A command without one gets guessed at. Examples are declared
    /// with the command's route, never generated: a generated example would
    /// restate the syntax help already shows and teach nothing about when to
    /// use the command.
    #[test]
    fn every_command_carries_a_worked_example() {
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
            "commands without a worked example; declare a `cli()` route with \
             `.with_examples(..)` keeping the current spelling: {bare:?}"
        );
    }

    /// Every command reaches the command line, and no two reach the same place.
    ///
    /// Both halves matter. A command with no spelling cannot be found by
    /// walking `--help`, which is the only way this surface is discoverable now
    /// that there is no `discover` tool. Two commands at one spelling means the
    /// tree silently serves one of them.
    #[test]
    fn every_command_has_exactly_one_spelling() {
        let mut by_spelling: std::collections::BTreeMap<String, Vec<&str>> =
            std::collections::BTreeMap::new();
        for contract in contracts() {
            by_spelling
                .entry(contract.spelling())
                .or_default()
                .push(contract.wire_name.as_str());
        }
        let collisions: Vec<(&String, &Vec<&str>)> = by_spelling
            .iter()
            .filter(|(_, names)| names.len() > 1)
            .collect();
        assert!(
            collisions.is_empty(),
            "these spellings serve more than one command; declare a `cli()` route \
             on all but one: {collisions:?}"
        );

        let unreachable: Vec<&str> = inventory::iter::<CommandDescriptor>
            .into_iter()
            .map(|desc| (desc.meta)())
            // A fixture is deliberately not part of anyone's command line, so
            // its absence is the design rather than an omission.
            .filter(|meta| !meta.path.starts_with("/test/"))
            // Internal worker plumbing is kept off the command line on purpose.
            .filter(|meta| !meta.is_internal())
            .map(|meta| meta.name)
            .filter(|name| !contracts().iter().any(|c| c.wire_name == *name))
            .collect();
        assert!(
            unreachable.is_empty(),
            "these commands have no spelling and could not derive one; declare a \
             `cli()` route: {unreachable:?}"
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
        // 292-command namespace had to forbid it. Now that every command has a
        // spelling the root covers the whole catalog, and it is still one
        // screen of nouns rather than a wall of commands.
        let text = tree().render_help("", None).expect("root help");
        let nouns = listed_commands(&text);

        for expected in ["agents", "mcp-servers", "sessions", "skills", "harnesses"] {
            assert!(
                nouns.iter().any(|n| n == expected),
                "{expected} missing:\n{text}"
            );
        }
        assert!(!nouns.iter().any(|noun| noun == "apps"), "{text}");
        assert!(
            nouns.len() < 60,
            "the root listed {} entries; it is supposed to be nouns, not commands:\n{text}",
            nouns.len()
        );
        // A verb at the root would mean a command escaped its noun.
        for verb in ["list", "get", "create", "delete"] {
            assert!(
                !nouns.iter().any(|n| n == verb),
                "verb {verb} at root:\n{text}"
            );
        }
    }

    #[test]
    fn node_help_lists_direct_children_only() {
        let listed = listed_commands(&tree().render_help("agents", None).unwrap());
        assert!(listed.contains(&"list".to_string()), "{listed:?}");
        assert!(listed.contains(&"exposures".to_string()), "{listed:?}");
        // A grandchild verb belongs to `agents exposures`, not to `agents`.
        assert!(!listed.contains(&"suspend".to_string()), "{listed:?}");
    }

    #[test]
    fn leaf_help_carries_flags_examples_and_the_wire_name() {
        let text = tree().render_help("agents list", None).expect("leaf help");
        // The usage line reads back what the caller typed, not the alias.
        assert!(text.contains("Usage: everruns agents list"), "{text}");
        assert!(text.contains("Examples:"), "{text}");
        assert!(text.contains("everruns agents list --search"), "{text}");
        assert!(text.contains("Wire name: list_agents"), "{text}");
    }

    #[test]
    fn unknown_verb_help_is_an_error_naming_real_neighbours() {
        let error = tree()
            .render_help("agents", Some("lst"))
            .expect_err("should be an error");
        assert!(error.contains("unknown command `lst`"), "{error}");
        assert!(error.contains("list"), "{error}");
    }

    #[test]
    fn soft_and_hard_delete_stay_separate_verbs() {
        // They carry different policies (MANAGE vs DANGEROUS), so the tree
        // must not collapse them behind one spelling plus a flag.
        for (line, expected) in [
            ("agents delete agt_1", "delete_agent"),
            ("agents destroy agt_1", "destroy_agent"),
        ] {
            let argv: Vec<&str> = line.split(' ').collect();
            match mapper().resolve(&argv) {
                Resolution::Run { wire_name, .. } => assert_eq!(wire_name, expected),
                other => panic!("{line}: {other:?}"),
            }
        }
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
            assert_eq!(leaf.wire_name, (desc.meta)().name);
        }
        assert!(
            seen.len() >= 50,
            "tranche should be declared: {}",
            seen.len()
        );
    }

    #[test]
    fn no_route_shadows_a_flat_command_name() {
        // A noun spelled like a flat command (`everruns list_agents ...`) would
        // read as that command in help and in scripts, and mean something else.
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
        let text = tree()
            .render_help("mcp-servers create", None)
            .expect("leaf help");

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
    use crate::records::FeatureFlags;

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

/// The tree must not become a route around the read-only toolset.
#[cfg(test)]
mod read_only_tests {
    use super::*;
    use crate::domains::common::CommandDescriptor;
    use crate::services::command_catalog::catalog::ToolsetMode;

    /// THREAT[TM-MCP-002]: `query` exposes read-only commands only. The
    /// `everruns` builtin resolves a spelling to a wire name and then asks the
    /// catalog whether that name may run in the toolset's mode, so a tree leaf
    /// must map to the command's own wire name for mode gating to stay the
    /// single decision point.
    #[test]
    fn every_tree_leaf_maps_to_a_registered_command_with_honest_read_only_status() {
        let mutating: Vec<&str> = inventory::iter::<CommandDescriptor>
            .into_iter()
            .filter_map(|desc| {
                let route = (desc.cli)()?;
                let meta = (desc.meta)();
                // The resolved name must be the command's own wire name...
                let leaf = tree().leaf(&route.spelling()).expect("declared leaf");
                assert_eq!(leaf.wire_name, meta.name);
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
