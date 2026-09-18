//! Rendering a node's help from the tree's shape.

use super::*;

/// Render help for a tree path.
///
/// Every response is bounded by the tree's shape rather than by a cap: the
/// root lists nouns, a node lists its children, a leaf lists its own flags.
pub fn render_help(tree: &CliTree, path: &str, unknown: Option<&str>) -> Result<String, String> {
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
        // The leaf's own parser renders its help, whichever adapter asked.
        // Help needs no raw argv, so a host that cannot reach clap to *parse*
        // can still reach it to explain, and both adapters describe a command
        // in exactly the same words.
        return Ok(leaf
            .parser
            .clone()
            .name(format!("{root} {path}"))
            .render_long_help()
            .to_string());
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
