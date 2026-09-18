//! Commands the CLI serves straight from the shared contract.
//!
//! The control plane routes fifty commands; this CLI hand-writes a dozen of
//! them. The rest were not missing because they are hard, but because each one
//! meant writing an arg struct whose fields already exist as the parameter type
//! the command deserializes, plus a body-building function whose shape is
//! already declared as an HTTP method and path.
//!
//! So they are mounted instead. Every contract carries its `method` and its
//! `http_path`, and this CLI already has a generic API client, which is enough
//! to run any of them without a hand-written implementation. What that buys is
//! not brevity: it is that `everruns agents versions rollback` exists for a
//! person for the same reason it exists for an agent, and stays spelled the
//! same way, because there is one declaration behind both.
//!
//! # Where the CLI still wins
//!
//! A hand-written command is kept whenever it exists. `everruns agents create`
//! reads a file, normalizes TOML the import endpoint cannot parse, and walks a
//! directory into `initial_files` — work no catalog command can do, since the
//! catalog runs on the server and the files are here. Mounting would replace
//! that with something strictly worse, so a contract whose spelling the CLI
//! already defines is skipped, and the shipped command line does not move.

use anyhow::{Context, Result, bail};
use clap::{ArgMatches, Command};
use everruns_cli_contract::ContractCommand;
use serde_json::{Map, Value};

use crate::commands::api::ApiClient;
use crate::output::OutputFormat;

/// Contract commands this CLI does not already spell by hand.
pub fn mounted(root: &Command) -> Vec<&'static ContractCommand> {
    everruns_cli_contract::commands()
        .iter()
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
pub fn augment(mut root: Command) -> Command {
    for contract in mounted(&root) {
        let leaf = contract.clap_command(&contract.verb);
        root = insert(root, &contract.path, leaf);
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

/// Run the command the caller typed, if it is one of the mounted contracts.
///
/// Returns `None` when the caller typed one of the CLI's hand-written
/// commands, which stays the caller's own code path.
pub async fn dispatch(
    root: &Command,
    matches: &ArgMatches,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
) -> Option<Result<()>> {
    let (path, leaf) = resolved(matches);
    let (verb, nouns) = path.split_last()?;

    let contract = mounted(root)
        .into_iter()
        .find(|contract| contract.path == nouns && &contract.verb == verb)?;

    Some(run(contract, leaf, api_url, api_key, org_id, output).await)
}

async fn run(
    contract: &ContractCommand,
    matches: &ArgMatches,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
) -> Result<()> {
    let params = everruns_cli_contract::params_from(contract, matches);
    let mut body = params.as_object().cloned().unwrap_or_default();

    // Path placeholders are consumed from the parameters, so a value never
    // travels twice: `/v1/agents/{id}` takes `id` out of the body.
    let path = fill_path(&contract.http_path, &mut body)?;

    let client = ApiClient::new(api_url, api_key, org_id);
    let response = match contract.method {
        ref method if method == "GET" => client.get(&with_query(&path, &body)).await,
        ref method if method == "DELETE" => client.delete(&with_query(&path, &body)).await,
        ref method if method == "POST" => client.post(&path, Some(&Value::Object(body))).await,
        ref method if method == "PATCH" => client.patch(&path, &Value::Object(body)).await,
        ref method if method == "PUT" => {
            // The generic client has no PUT; upsert-style commands are the only
            // users, and they are better served by saying so than by silently
            // sending a POST that creates a duplicate.
            bail!(
                "`{}` is a PUT and this CLI's client does not send PUT yet",
                contract.spelling()
            )
        }
        ref method => bail!(
            "unsupported method `{method}` for `{}`",
            contract.spelling()
        ),
    }?;

    print(&response, output);
    Ok(())
}

/// Substitute `{field}` segments, removing each from the body as it is used.
fn fill_path(template: &str, body: &mut Map<String, Value>) -> Result<String> {
    let mut path = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        let close = rest[open..]
            .find('}')
            .map(|offset| open + offset)
            .with_context(|| format!("unterminated placeholder in `{template}`"))?;
        path.push_str(&rest[..open]);

        let field = &rest[open + 1..close];
        let value = body
            .remove(field)
            .with_context(|| format!("`{template}` needs `--{}`", field.replace('_', "-")))?;
        path.push_str(&as_text(&value));
        rest = &rest[close + 1..];
    }
    path.push_str(rest);
    Ok(path)
}

/// Append leftover parameters as a query string, for methods with no body.
fn with_query(path: &str, params: &Map<String, Value>) -> String {
    if params.is_empty() {
        return path.to_string();
    }
    let query = params
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(&as_text(value))))
        .collect::<Vec<_>>()
        .join("&");
    format!("{path}?{query}")
}

/// A value as it belongs in a path segment or a query value. A string is
/// itself; anything else is its JSON, so a list or an object survives.
fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Percent-encode everything outside the unreserved set, so a search term with
/// a space or an ampersand cannot rewrite the query it travels in.
fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
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
