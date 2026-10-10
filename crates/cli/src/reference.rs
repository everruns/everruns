//! The CLI reference pages on docs.everruns.com, generated from this binary.
//!
//! Decision: the pages are rendered from the same clap tree `everruns --help`
//! walks (hand-written commands plus the mounted contract), so a page can
//! never describe a flag the binary does not have. They exist for two
//! readers. A person browsing the docs gets one page per command group with a
//! short table to skim and a section per command. An agent outside our plugin
//! that only has web search lands on the exact flags in one search instead of
//! guessing, because no model has this command line in its training data.
//!
//! Each page is written for a person first: the group's one-line summary, a
//! "what each command does" table, then usage, flags and a worked example per
//! command. Wire names and other agent plumbing stay out.

use std::collections::BTreeMap;

use clap::{Arg, ArgAction, Command};
use everruns_cli_contract::Mapper;

/// Directory, relative to the repository root, that holds the pages.
pub const DIR: &str = "docs/reference/cli";

/// Every page, by file name, for the CLI tree `root`.
pub fn pages(root: &Command) -> BTreeMap<String, String> {
    let mut pages = BTreeMap::new();
    let mut groups = Vec::new();
    let mut top_level = Vec::new();
    let mut children: Vec<&Command> = visible(root).collect();
    children.sort_by_key(|child| child.get_name());
    for child in children {
        if visible(child).next().is_some() {
            pages.insert(format!("{}.md", child.get_name()), group_page(child));
            groups.push(child);
        } else {
            top_level.push(child);
        }
    }
    pages.insert("index.md".to_string(), index_page(&groups, &top_level));
    pages
}

fn visible(command: &Command) -> impl Iterator<Item = &Command> {
    command
        .get_subcommands()
        .filter(|child| !child.is_hide_set() && child.get_name() != "help")
}

fn index_page(groups: &[&Command], top_level: &[&Command]) -> String {
    let mut page = String::new();
    page.push_str(
        "---\n\
         title: CLI command reference\n\
         description: Every everruns CLI command, grouped by what it manages, with flags and a worked example.\n\
         sidebar:\n  label: Overview\n  order: 0\n\
         appliesTo: [platform, cloud]\n\
         ---\n\n",
    );
    page.push_str(GENERATED);
    page.push_str(
        "Every command below is also in `everruns --help`, and an agent running on \
         Everruns types the same words in its shell. Install the CLI and sign in \
         first; see [CLI](/features/cli/).\n\n\
         ## Flags every command takes\n\n\
         | Flag | Description |\n|---|---|\n\
         | `--reason <TEXT>` | Why you are making this change. Recorded in the changed entity's history; give one on every command that changes something. |\n\
         | `--context-revision <N>` | The revision of the entity's manager notes you read (`everruns context get`). The change is refused if the notes changed since. |\n\
         | `-o`, `--output <FORMAT>` | `text` (default), `json` or `yaml`. |\n\
         | `-q`, `--quiet` | Print only the essential identifier, for capturing ids in scripts. |\n\n",
    );
    if !top_level.is_empty() {
        page.push_str("## Session and account\n\n| Command | What it does |\n|---|---|\n");
        for command in top_level {
            page.push_str(&format!(
                "| `everruns {}` | {} |\n",
                command.get_name(),
                cell(&about(command))
            ));
        }
        page.push('\n');
    }
    page.push_str("## Command groups\n\n| Group | What it manages |\n|---|---|\n");
    for group in groups {
        page.push_str(&format!(
            "| [`{name}`](/reference/cli/{name}/) | {about} |\n",
            name = group.get_name(),
            about = cell(&about(group))
        ));
    }
    page
}

fn group_page(group: &Command) -> String {
    let name = group.get_name();
    let summary = about(group);
    let mut leaves = Vec::new();
    collect_leaves(group, name, &mut leaves);

    let mut page = format!(
        "---\ntitle: everruns {name}\ndescription: {description}\nsidebar:\n  label: {name}\nappliesTo: [platform, cloud]\n---\n\n",
        description = yaml_line(&format!(
            "{} CLI reference for everruns {name}.",
            sentence(&summary)
        )),
    );
    page.push_str(GENERATED);
    page.push_str(&format!("{}\n\n", sentence(&summary)));
    page.push_str("| Command | What it does |\n|---|---|\n");
    for (spelling, command) in &leaves {
        page.push_str(&format!(
            "| [`{spelling}`](#{anchor}) | {about} |\n",
            anchor = anchor(spelling),
            about = cell(&first_sentence(&about(command)))
        ));
    }
    for (spelling, command) in &leaves {
        page.push_str(&command_section(spelling, command));
    }
    page
}

fn collect_leaves<'a>(command: &'a Command, path: &str, out: &mut Vec<(String, &'a Command)>) {
    let mut children = visible(command).peekable();
    if children.peek().is_none() {
        out.push((path.to_string(), command));
        return;
    }
    for child in children {
        collect_leaves(child, &format!("{path} {}", child.get_name()), out);
    }
}

fn command_section(spelling: &str, command: &Command) -> String {
    let mut section = format!("\n## {spelling}\n\n");
    let description = command
        .get_long_about()
        .or(command.get_about())
        .map(|text| text.to_string())
        .unwrap_or_default();
    if !description.trim().is_empty() {
        section.push_str(&format!("{}\n\n", sentence(description.trim())));
    }

    let usage = command
        .clone()
        .bin_name(format!("everruns {spelling}"))
        .render_usage()
        .to_string();
    let usage = usage.trim().trim_start_matches("Usage:").trim();
    section.push_str(&format!("```bash\n{usage}\n```\n\n"));

    let args: Vec<&Arg> = command
        .get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .filter(|arg| !matches!(arg.get_id().as_str(), "help" | "version"))
        // Every command takes these; the overview page explains them once
        // rather than repeating two identical rows under every command.
        .filter(|arg| !matches!(arg.get_long(), Some("reason" | "context-revision")))
        // A contract argument that is also a bare word is listed twice in
        // clap (the flag and the positional); the flag row says both.
        .filter(|arg| !(arg.is_positional() && arg.get_id().as_str().contains('\u{1}')))
        .collect();
    if !args.is_empty() {
        section.push_str("| Flag | Description |\n|---|---|\n");
        for arg in args {
            section.push_str(&format!(
                "| {} | {} |\n",
                spelling_of(arg),
                cell(&describe(arg))
            ));
        }
        section.push('\n');
    }

    let examples = examples(spelling, command);
    if !examples.is_empty() {
        section.push_str("Example:\n\n```bash\n");
        section.push_str(&examples.join("\n\n"));
        section.push_str("\n```\n");
    }
    section
}

/// Worked examples as `# intent` plus the command line.
///
/// Contract commands carry them as data; a hand-written command keeps its
/// own `after_help` text, which is shown as written.
fn examples(spelling: &str, command: &Command) -> Vec<String> {
    let mapper = Mapper::everruns();
    if let Some(contract) = mapper
        .contracts()
        .find(|contract| contract.spelling() == spelling)
        && contract_mounted(command)
    {
        return contract
            .examples
            .iter()
            .map(|example| {
                format!(
                    "# {}\neverruns {}",
                    example.intent,
                    strip_root(&example.command)
                )
            })
            .collect();
    }
    command
        .get_after_help()
        .map(|text| text.to_string())
        .filter(|text| !text.trim().is_empty())
        .map(|text| vec![text.trim().to_string()])
        .unwrap_or_default()
}

/// Whether this leaf is the contract's parser rather than a hand-written one:
/// contract parsers always end their help with the wire name.
fn contract_mounted(command: &Command) -> bool {
    command
        .get_after_help()
        .is_some_and(|text| text.to_string().contains("Wire name: "))
}

fn strip_root(command: &str) -> &str {
    command.strip_prefix("everruns ").unwrap_or(command)
}

fn spelling_of(arg: &Arg) -> String {
    let value = arg
        .get_value_names()
        .and_then(|names| names.first())
        .map(|name| name.to_string())
        .unwrap_or_else(|| arg.get_id().as_str().to_uppercase().replace('-', "_"));
    let takes_value = !matches!(
        arg.get_action(),
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
    ) && arg
        .get_num_args()
        .is_none_or(|range| range.max_values() > 0);
    let mut parts = Vec::new();
    if let Some(short) = arg.get_short() {
        parts.push(format!("`-{short}`"));
    }
    if let Some(long) = arg.get_long() {
        let optional_value = arg
            .get_num_args()
            .is_some_and(|range| range.min_values() == 0);
        if takes_value && !optional_value {
            parts.push(format!("`--{long} <{value}>`"));
        } else {
            parts.push(format!("`--{long}`"));
        }
    }
    if arg.is_positional() {
        parts.push(format!("`<{value}>`"));
    }
    parts.join(", ")
}

fn describe(arg: &Arg) -> String {
    let mut text = arg
        .get_long_help()
        .or(arg.get_help())
        .map(|help| sentence(help.to_string().trim()))
        .unwrap_or_default();
    let choices: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| format!("`{}`", value.get_name()))
        .collect();
    // Boolean flags list true/false as possible values; that is noise.
    if !choices.is_empty() && choices != ["`true`", "`false`"] {
        push_part(&mut text, &format!("One of {}.", choices.join(", ")));
    }
    if matches!(arg.get_action(), ArgAction::Append) {
        push_part(&mut text, "Repeatable.");
    }
    if arg.is_required_set() {
        text = if text.is_empty() {
            "Required.".to_string()
        } else {
            format!("Required. {text}")
        };
    }
    text
}

fn push_part(text: &mut String, part: &str) {
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(part);
}

fn about(command: &Command) -> String {
    command
        .get_about()
        .map(|text| text.to_string())
        .unwrap_or_default()
}

/// Text as a sentence: capitalised, ending in a full stop.
fn sentence(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let mut chars = text.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let mut out = format!("{first}{}", chars.as_str());
    if !out.ends_with(['.', '!', '?', ')', ':']) {
        out.push('.');
    }
    out
}

fn first_sentence(text: &str) -> String {
    let text = sentence(text);
    match text.find(". ") {
        Some(end) => text[..=end].to_string(),
        None => text,
    }
}

/// A table cell: one line, pipes escaped.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn yaml_line(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Starlight's heading slug for `agents triggers create`.
fn anchor(spelling: &str) -> String {
    spelling.replace(' ', "-")
}

const GENERATED: &str = "<!-- Generated from the everruns CLI by \
`UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. \
Edit command descriptions and examples in the code, not here. -->\n\n";
