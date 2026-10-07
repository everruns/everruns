//! Commands the CLI serves straight from the shared contract.
//!
//! Decision: every contract command runs through one generic path. The words
//! are resolved by the same mapper every other surface uses
//! (`everruns_cli_contract::Mapper`), and the command is sent to
//! `POST /v1/commands/{wire_name}`, which runs it through the same pipeline as
//! MCP `execute` and the Platform capability. No command needs a hand-written
//! client, an HTTP method or a path template here, so `everruns history
//! restore` exists for a person for the same reason it exists for an agent,
//! spelled the same way and validated the same way.
//!
//! # Where the CLI still writes its own
//!
//! Only where the work happens on this machine: logging in, syncing files,
//! streaming a session, and commands that read or write local files in a way a
//! flag value cannot express (`agents import <dir>`, `agents export --out`).
//! A contract whose spelling the CLI already defines is not mounted, so the
//! hand-written command wins.
//!
//! # Local files
//!
//! A text or JSON argument written `@path` is read from that local file, so
//! `everruns skills create --skill-md @SKILL.md` sends the file's contents. A
//! value that really starts with `@` is written `@@`.

use anyhow::{Context, Result};
use clap::{ArgMatches, Command, CommandFactory};
use everruns_cli_contract::{ArgKind, ContractCommand, Mapper};
use serde_json::{Value, json};

use crate::commands::api::ApiClient;
use crate::output::OutputFormat;

/// Contract commands this CLI does not already spell by hand.
pub fn mounted(root: &Command) -> Vec<&'static ContractCommand> {
    Mapper::everruns()
        .contracts()
        .filter(|contract| !defines(root, &contract.path, &contract.verb))
        .collect()
}

/// Whether the CLI's own tree already defines this spelling.
fn defines(root: &Command, path: &[String], verb: &str) -> bool {
    let mut node = root;
    for segment in path {
        match node.find_subcommand(segment) {
            Some(child) => node = child,
            None => return false,
        }
    }
    node.find_subcommand(verb).is_some()
}

/// Mount every contract command the CLI does not already spell.
///
/// Each leaf is the shared tree's own parser, so the flags, positionals and
/// help a person sees here are the ones an agent sees in its shell.
pub fn augment(mut root: Command) -> Command {
    let tree = Mapper::everruns().tree();
    for contract in mounted(&root) {
        let Some(leaf) = tree.leaf(&contract.spelling()) else {
            continue;
        };
        root = insert(
            root,
            &contract.path,
            leaf.parser.clone().name(&contract.verb),
        );
    }
    for (path, about) in everruns_cli_contract::mapper::NODE_ABOUT {
        let words: Vec<String> = path.split(' ').map(ToOwned::to_owned).collect();
        root = describe(root, &words, about);
    }
    root
}

fn insert(node: Command, path: &[String], leaf: Command) -> Command {
    let Some((head, rest)) = path.split_first() else {
        return node.subcommand(leaf);
    };
    if node.find_subcommand(head).is_some() {
        node.mut_subcommand(head, |child| insert(child, rest, leaf))
    } else {
        let created = Command::new(head.clone())
            .about(format!("{head} commands"))
            .subcommand_required(true)
            .arg_required_else_help(true);
        node.subcommand(insert(created, rest, leaf))
    }
}

/// Give a node the tree's description, where the CLI did not write its own.
fn describe(node: Command, path: &[String], about: &str) -> Command {
    let Some((head, rest)) = path.split_first() else {
        return node;
    };
    if node.find_subcommand(head).is_none() {
        return node;
    }
    node.mut_subcommand(head, |child| {
        if rest.is_empty() {
            let generic = format!("{head} commands");
            match child.get_about().map(|text| text.to_string()) {
                Some(text) if text != generic => child,
                _ => child.about(about.to_string()),
            }
        } else {
            describe(child, rest, about)
        }
    })
}

/// The subcommand path the caller typed, and the matches at its leaf.
fn resolved(matches: &ArgMatches) -> (Vec<String>, &ArgMatches) {
    let mut path = Vec::new();
    let mut node = matches;
    while let Some((name, child)) = node.subcommand() {
        path.push(name.to_string());
        node = child;
    }
    (path, node)
}

/// The mounted contract the caller typed, with the matches at its leaf.
///
/// `None` means a hand-written command, which stays the caller's own code
/// path. Decided against the CLI's own tree, not the augmented one: in the
/// augmented tree every mounted spelling looks defined, so nothing would ever
/// dispatch.
pub fn selected(matches: &ArgMatches) -> Option<(&'static ContractCommand, &ArgMatches)> {
    let (path, leaf) = resolved(matches);
    let (verb, nouns) = path.split_last()?;
    let contract = mounted(&crate::Cli::command())
        .into_iter()
        .find(|contract| contract.path == nouns && &contract.verb == verb)?;
    Some((contract, leaf))
}

/// Run a mounted contract command through `POST /v1/commands/{wire_name}`.
pub async fn dispatch(
    contract: &ContractCommand,
    matches: &ArgMatches,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
) -> Result<()> {
    let client = ApiClient::new(api_url, api_key, org_id);
    run(&client, contract, matches, output).await
}

async fn run(
    client: &ApiClient<'_>,
    contract: &ContractCommand,
    matches: &ArgMatches,
    output: OutputFormat,
) -> Result<()> {
    let mut params = everruns_cli_contract::params_from(contract, matches);
    read_local_files(contract, &mut params)?;
    let response = execute(client, &contract.wire_name, params).await?;
    print(&response, output);
    Ok(())
}

/// Run one command by wire name and return its output, printing any warnings
/// the server attached to stderr.
pub async fn execute(client: &ApiClient<'_>, wire_name: &str, mut params: Value) -> Result<Value> {
    // `--reason` is invocation metadata: it goes in the envelope, where the
    // server records it in the entity's history, not among the params.
    let reason = params
        .as_object_mut()
        .and_then(|object| object.remove(everruns_cli_contract::REASON_FIELD));
    let context_revision = params
        .as_object_mut()
        .and_then(|object| object.remove(everruns_cli_contract::CONTEXT_REVISION_FIELD));
    let mut body = json!({
        "params": params,
        "metadata": { "client": concat!("everruns-cli/", env!("CARGO_PKG_VERSION")) },
    });
    if let Some(reason) = reason {
        body["reason"] = reason;
    }
    if let Some(revision) = context_revision {
        body["context_revision"] = revision;
    }
    let response = client
        .post_command(
            &format!("/v1/commands/{wire_name}"),
            &body,
            &idempotency_key(),
        )
        .await?;
    if let Some(warnings) = response.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str) {
            eprintln!("warning: {warning}");
        }
    }
    Ok(response.get("output").cloned().unwrap_or(Value::Null))
}

/// The key that makes one invocation's command safe to retry: the caller's
/// `EVERRUNS_IDEMPOTENCY_KEY` when a script re-runs the CLI and wants the
/// server to recognize the repeat, otherwise a fresh one per invocation.
fn idempotency_key() -> String {
    std::env::var("EVERRUNS_IDEMPOTENCY_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| format!("cli-{}", uuid::Uuid::new_v4().simple()))
}

/// Replace `@path` values with the named file's contents.
fn read_local_files(contract: &ContractCommand, params: &mut Value) -> Result<()> {
    let Some(object) = params.as_object_mut() else {
        return Ok(());
    };
    for arg in &contract.args {
        if !matches!(arg.kind, ArgKind::String | ArgKind::Json) {
            continue;
        }
        let Some(Value::String(text)) = object.get(&arg.field) else {
            continue;
        };
        if let Some(value) = local_value(text, arg.kind)? {
            object.insert(arg.field.clone(), value);
        }
    }
    Ok(())
}

fn local_value(text: &str, kind: ArgKind) -> Result<Option<Value>> {
    if let Some(literal) = text.strip_prefix("@@") {
        return Ok(Some(Value::String(format!("@{literal}"))));
    }
    let Some(path) = text.strip_prefix('@').filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("could not read `{path}`"))?;
    Ok(Some(match kind {
        // A document the server parses: send it structured when it is JSON,
        // as text otherwise so the command's own error explains what is wrong.
        ArgKind::Json => serde_json::from_str(&contents).unwrap_or(Value::String(contents)),
        _ => Value::String(contents),
    }))
}

fn print(response: &Value, output: OutputFormat) {
    match output {
        // A generic command has no hand-written table, and inventing one from
        // an unknown shape reads worse than the document itself. Pretty JSON is
        // legible to a person and still parses for `jq`.
        OutputFormat::Text | OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(response).unwrap_or_else(|_| response.to_string())
            );
        }
        OutputFormat::Yaml => match serde_yaml::to_string(response) {
            Ok(text) => print!("{text}"),
            Err(_) => println!("{response}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_at_path_reads_the_file() {
        let dir = std::env::temp_dir().join(format!("everruns-cli-at-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("SKILL.md");
        std::fs::write(&file, "---\nname: x\n---\nbody").unwrap();
        let value = local_value(&format!("@{}", file.display()), ArgKind::String)
            .unwrap()
            .unwrap();
        assert_eq!(value, Value::String("---\nname: x\n---\nbody".into()));

        let doc = dir.join("caps.json");
        std::fs::write(&doc, r#"[{"ref":"web"}]"#).unwrap();
        let value = local_value(&format!("@{}", doc.display()), ArgKind::Json)
            .unwrap()
            .unwrap();
        assert_eq!(value, json!([{ "ref": "web" }]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_doubled_at_is_a_literal_and_plain_text_is_untouched() {
        assert_eq!(
            local_value("@@handle", ArgKind::String).unwrap(),
            Some(Value::String("@handle".into()))
        );
        assert_eq!(local_value("plain", ArgKind::String).unwrap(), None);
        assert_eq!(local_value("@", ArgKind::String).unwrap(), None);
    }

    #[test]
    fn a_missing_file_names_the_path() {
        let error = local_value("@/no/such/file", ArgKind::String).unwrap_err();
        assert!(error.to_string().contains("/no/such/file"), "{error}");
    }

    #[test]
    fn every_mounted_leaf_is_the_shared_trees_parser() {
        let root = augment(crate::Cli::command());
        let update = root
            .find_subcommand("agents")
            .and_then(|agents| agents.find_subcommand("triggers"))
            .and_then(|triggers| triggers.find_subcommand("update"))
            .expect("agents triggers update is mounted");
        assert!(
            update
                .clone()
                .render_long_help()
                .to_string()
                .contains("Wire name: update_agent_trigger")
        );
    }

    /// Regression: the selection used to be checked against the augmented
    /// tree, where every mounted spelling is defined, so no contract command
    /// ever dispatched.
    #[test]
    fn a_mounted_command_is_selected_and_a_hand_written_one_is_not() {
        let root = augment(crate::Cli::command());
        let matches = root
            .clone()
            .try_get_matches_from([
                "everruns",
                "agents",
                "triggers",
                "list",
                "--agent-id",
                "a_1",
            ])
            .unwrap();
        let (contract, leaf) = selected(&matches).expect("a mounted command");
        assert_eq!(contract.wire_name, "list_agent_triggers");
        assert_eq!(
            everruns_cli_contract::params_from(contract, leaf),
            json!({ "agent_id": "a_1" })
        );

        let matches = root
            .try_get_matches_from(["everruns", "agents", "create", "--name", "x"])
            .unwrap();
        assert!(selected(&matches).is_none());
    }
}
