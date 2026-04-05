// Structural Outlines (EVE-248)
//
// Extracts function/class/struct/method signatures from source files using
// tree-sitter. When read_file truncates content, the outline gives agents
// structural context for the unread portions.
//
// Design decisions:
// - Plain-text output appended to content (simplest, matches Copilot UX)
// - Only parse when content is truncated (no overhead for small files)
// - Phase 1: Rust, TypeScript/JavaScript, Python
// - Outline items are one-level nested max (methods inside impl/class)

/// A single outline item extracted from a source file.
#[derive(Debug, Clone)]
pub struct OutlineItem {
    /// e.g. "fn", "struct", "impl", "class", "def", "mod"
    pub kind: &'static str,
    /// Name of the declaration
    pub name: String,
    /// Full signature text (e.g. "fn process(input: &Input) -> Result<Output>")
    pub signature: String,
    /// 1-based line number
    pub line: usize,
    /// Nested items (methods inside impl/class)
    pub children: Vec<OutlineItem>,
}

/// Generate a structural outline for the given source file content.
/// Returns outline items sorted by line number. Returns empty vec if
/// the language is not supported or parsing fails.
#[cfg(feature = "tree-sitter-outlines")]
pub fn generate_outline(content: &str, path: &str) -> Vec<OutlineItem> {
    let lang = match detect_language(path) {
        Some(l) => l,
        None => return vec![],
    };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&lang).is_err() {
        return vec![];
    }

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return vec![],
    };

    let lang_id = language_id(path);
    extract_outline(tree.root_node(), content, lang_id)
}

#[cfg(not(feature = "tree-sitter-outlines"))]
pub fn generate_outline(_content: &str, _path: &str) -> Vec<OutlineItem> {
    vec![]
}

/// Format outline items as plain text for appending to read_file content.
/// Only includes items outside the shown window [start_line, end_line] (1-based).
pub fn format_outline(
    items: &[OutlineItem],
    start_line: usize,
    end_line: usize,
    total_lines: usize,
) -> Option<String> {
    // Collect items outside the shown range
    let before: Vec<&OutlineItem> = items.iter().filter(|i| i.line < start_line).collect();
    let after: Vec<&OutlineItem> = items.iter().filter(|i| i.line > end_line).collect();

    if before.is_empty() && after.is_empty() {
        return None;
    }

    let mut result = String::new();

    if !before.is_empty() {
        result.push_str(&format!(
            "\n--- Outline of lines 1-{} (not shown) ---\n",
            start_line - 1
        ));
        for item in &before {
            format_item(&mut result, item, 0);
        }
    }

    if !after.is_empty() {
        result.push_str(&format!(
            "\n--- Outline of lines {}-{} (not shown) ---\n",
            end_line + 1,
            total_lines
        ));
        for item in &after {
            format_item(&mut result, item, 0);
        }
    }

    Some(result)
}

fn format_item(out: &mut String, item: &OutlineItem, indent: usize) {
    let pad = "  ".repeat(indent);
    out.push_str(&format!(
        "{pad}// L{}: {} {{ ... }}\n",
        item.line, item.signature
    ));
    for child in &item.children {
        format_item(out, child, indent + 1);
    }
}

// --- Language detection ---

#[cfg(feature = "tree-sitter-outlines")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LangId {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
}

#[cfg(feature = "tree-sitter-outlines")]
fn language_id(path: &str) -> LangId {
    let lower = path.to_lowercase();
    if lower.ends_with(".rs") {
        LangId::Rust
    } else if lower.ends_with(".tsx") {
        LangId::Tsx
    } else if lower.ends_with(".ts") {
        LangId::TypeScript
    } else if lower.ends_with(".jsx")
        || lower.ends_with(".js")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
    {
        LangId::JavaScript
    } else if lower.ends_with(".py") {
        LangId::Python
    } else {
        LangId::Rust // fallback, won't match
    }
}

#[cfg(feature = "tree-sitter-outlines")]
fn detect_language(path: &str) -> Option<tree_sitter::Language> {
    let lower = path.to_lowercase();
    if lower.ends_with(".rs") {
        Some(tree_sitter_rust::LANGUAGE.into())
    } else if lower.ends_with(".ts") || lower.ends_with(".mts") {
        Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
    } else if lower.ends_with(".tsx") {
        Some(tree_sitter_typescript::LANGUAGE_TSX.into())
    } else if lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
    {
        Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()) // TS parser handles JS
    } else if lower.ends_with(".py") {
        Some(tree_sitter_python::LANGUAGE.into())
    } else {
        None
    }
}

// --- Outline extraction ---

#[cfg(feature = "tree-sitter-outlines")]
fn extract_outline(root: tree_sitter::Node, source: &str, lang: LangId) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match lang {
            LangId::Rust => extract_rust_item(child, source, &mut items),
            LangId::TypeScript | LangId::Tsx | LangId::JavaScript => {
                extract_ts_item(child, source, &mut items)
            }
            LangId::Python => extract_python_item(child, source, &mut items),
        }
    }

    items
}

#[cfg(feature = "tree-sitter-outlines")]
fn node_text<'a>(node: tree_sitter::Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

#[cfg(feature = "tree-sitter-outlines")]
fn find_child_by_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

// --- Rust extraction ---

#[cfg(feature = "tree-sitter-outlines")]
fn extract_rust_item(node: tree_sitter::Node, source: &str, items: &mut Vec<OutlineItem>) {
    let kind = node.kind();
    let line = node.start_position().row + 1; // 1-based

    match kind {
        "function_item" => {
            if let Some(sig) = rust_fn_signature(node, source) {
                items.push(OutlineItem {
                    kind: "fn",
                    name: find_child_by_kind(node, "identifier")
                        .map(|n| node_text(n, source).to_string())
                        .unwrap_or_default(),
                    signature: sig,
                    line,
                    children: vec![],
                });
            }
        }
        "struct_item" => {
            let name = find_child_by_kind(node, "type_identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            items.push(OutlineItem {
                kind: "struct",
                name: name.clone(),
                signature: format!("struct {name}"),
                line,
                children: vec![],
            });
        }
        "enum_item" => {
            let name = find_child_by_kind(node, "type_identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            items.push(OutlineItem {
                kind: "enum",
                name: name.clone(),
                signature: format!("enum {name}"),
                line,
                children: vec![],
            });
        }
        "impl_item" => {
            let type_name = find_child_by_kind(node, "type_identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();

            // Check if it's a trait impl
            let trait_name = find_child_by_kind(node, "scoped_type_identifier")
                .or_else(|| {
                    // For `impl Trait for Type`, find the trait identifier
                    let mut cursor = node.walk();
                    node.children(&mut cursor).find(|c| {
                        c.kind() == "type_identifier"
                            && c.start_position().row == node.start_position().row
                    })
                })
                .map(|n| node_text(n, source).to_string());

            let sig = if let Some(ref t) = trait_name {
                if t != &type_name {
                    format!("impl {t} for {type_name}")
                } else {
                    format!("impl {type_name}")
                }
            } else {
                format!("impl {type_name}")
            };

            let mut children = Vec::new();
            if let Some(body) = find_child_by_kind(node, "declaration_list") {
                let mut cursor = body.walk();
                for method in body.children(&mut cursor) {
                    if method.kind() == "function_item"
                        && let Some(method_sig) = rust_fn_signature(method, source)
                    {
                        children.push(OutlineItem {
                            kind: "fn",
                            name: find_child_by_kind(method, "identifier")
                                .map(|n| node_text(n, source).to_string())
                                .unwrap_or_default(),
                            signature: method_sig,
                            line: method.start_position().row + 1,
                            children: vec![],
                        });
                    }
                }
            }

            items.push(OutlineItem {
                kind: "impl",
                name: type_name,
                signature: sig,
                line,
                children,
            });
        }
        "mod_item" => {
            let name = find_child_by_kind(node, "identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            // Check for #[cfg(test)]
            let is_test = node_text(node, source).contains("#[cfg(test)]");
            let prefix = if is_test { "#[cfg(test)] " } else { "" };
            items.push(OutlineItem {
                kind: "mod",
                name: name.clone(),
                signature: format!("{prefix}mod {name}"),
                line,
                children: vec![],
            });
        }
        "trait_item" => {
            let name = find_child_by_kind(node, "type_identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            items.push(OutlineItem {
                kind: "trait",
                name: name.clone(),
                signature: format!("trait {name}"),
                line,
                children: vec![],
            });
        }
        _ => {}
    }
}

#[cfg(feature = "tree-sitter-outlines")]
fn rust_fn_signature(node: tree_sitter::Node, source: &str) -> Option<String> {
    // Extract up to the opening brace
    let full = node_text(node, source);
    let sig = if let Some(brace_pos) = full.find('{') {
        full[..brace_pos].trim()
    } else {
        // For declarations without body (trait methods)
        if let Some(semi_pos) = full.find(';') {
            full[..semi_pos].trim()
        } else {
            full.lines().next().unwrap_or(full).trim()
        }
    };
    // Collapse whitespace
    let sig = sig.split_whitespace().collect::<Vec<_>>().join(" ");
    if sig.is_empty() { None } else { Some(sig) }
}

// --- TypeScript/JavaScript extraction ---

#[cfg(feature = "tree-sitter-outlines")]
fn extract_ts_item(node: tree_sitter::Node, source: &str, items: &mut Vec<OutlineItem>) {
    let kind = node.kind();
    let line = node.start_position().row + 1;

    match kind {
        "function_declaration" => {
            let name = find_child_by_kind(node, "identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            let sig = ts_fn_signature(node, source);
            items.push(OutlineItem {
                kind: "function",
                name,
                signature: sig,
                line,
                children: vec![],
            });
        }
        "class_declaration" => {
            let name = find_child_by_kind(node, "type_identifier")
                .or_else(|| find_child_by_kind(node, "identifier"))
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();

            let mut children = Vec::new();
            if let Some(body) = find_child_by_kind(node, "class_body") {
                let mut cursor = body.walk();
                for member in body.children(&mut cursor) {
                    if member.kind() == "method_definition"
                        || member.kind() == "public_field_definition"
                    {
                        let mname = find_child_by_kind(member, "property_identifier")
                            .map(|n| node_text(n, source).to_string())
                            .unwrap_or_default();
                        children.push(OutlineItem {
                            kind: "method",
                            name: mname.clone(),
                            signature: ts_fn_signature(member, source),
                            line: member.start_position().row + 1,
                            children: vec![],
                        });
                    }
                }
            }

            items.push(OutlineItem {
                kind: "class",
                name: name.clone(),
                signature: format!("class {name}"),
                line,
                children,
            });
        }
        "interface_declaration" => {
            let name = find_child_by_kind(node, "type_identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            items.push(OutlineItem {
                kind: "interface",
                name: name.clone(),
                signature: format!("interface {name}"),
                line,
                children: vec![],
            });
        }
        "type_alias_declaration" => {
            let name = find_child_by_kind(node, "type_identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            items.push(OutlineItem {
                kind: "type",
                name: name.clone(),
                signature: format!("type {name}"),
                line,
                children: vec![],
            });
        }
        "export_statement" => {
            // Recurse into exported declarations
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_ts_item(child, source, items);
            }
        }
        "lexical_declaration" => {
            // const foo = () => {} or const foo = function() {}
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator"
                    && let Some(value) = find_child_by_kind(child, "arrow_function")
                        .or_else(|| find_child_by_kind(child, "function"))
                {
                    let name = find_child_by_kind(child, "identifier")
                        .map(|n| node_text(n, source).to_string())
                        .unwrap_or_default();
                    let sig = ts_fn_signature(value, source);
                    items.push(OutlineItem {
                        kind: "function",
                        name,
                        signature: sig,
                        line: child.start_position().row + 1,
                        children: vec![],
                    });
                }
            }
        }
        _ => {}
    }
}

#[cfg(feature = "tree-sitter-outlines")]
fn ts_fn_signature(node: tree_sitter::Node, source: &str) -> String {
    let full = node_text(node, source);
    // Take up to opening brace
    if let Some(brace_pos) = full.find('{') {
        let sig = full[..brace_pos].trim();
        sig.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        full.lines().next().unwrap_or("").trim().to_string()
    }
}

// --- Python extraction ---

#[cfg(feature = "tree-sitter-outlines")]
fn extract_python_item(node: tree_sitter::Node, source: &str, items: &mut Vec<OutlineItem>) {
    let kind = node.kind();
    let line = node.start_position().row + 1;

    match kind {
        "function_definition" => {
            let name = find_child_by_kind(node, "identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            let sig = python_fn_signature(node, source);
            items.push(OutlineItem {
                kind: "def",
                name,
                signature: sig,
                line,
                children: vec![],
            });
        }
        "class_definition" => {
            let name = find_child_by_kind(node, "identifier")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();

            let mut children = Vec::new();
            if let Some(body) = find_child_by_kind(node, "block") {
                let mut cursor = body.walk();
                for member in body.children(&mut cursor) {
                    if member.kind() == "function_definition" {
                        let mname = find_child_by_kind(member, "identifier")
                            .map(|n| node_text(n, source).to_string())
                            .unwrap_or_default();
                        children.push(OutlineItem {
                            kind: "def",
                            name: mname,
                            signature: python_fn_signature(member, source),
                            line: member.start_position().row + 1,
                            children: vec![],
                        });
                    }
                }
            }

            items.push(OutlineItem {
                kind: "class",
                name: name.clone(),
                signature: format!("class {name}"),
                line,
                children,
            });
        }
        "decorated_definition" => {
            // Recurse into the actual definition
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "function_definition" || child.kind() == "class_definition" {
                    extract_python_item(child, source, items);
                }
            }
        }
        _ => {}
    }
}

#[cfg(feature = "tree-sitter-outlines")]
fn python_fn_signature(node: tree_sitter::Node, source: &str) -> String {
    let full = node_text(node, source);
    // Take the first line (def foo(...):)
    let first = full.lines().next().unwrap_or("").trim();
    // Remove trailing colon
    let sig = first.strip_suffix(':').unwrap_or(first);
    sig.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_outline_empty() {
        assert!(format_outline(&[], 1, 100, 100).is_none());
    }

    #[test]
    fn test_format_outline_after_only() {
        let items = vec![OutlineItem {
            kind: "fn",
            name: "process".to_string(),
            signature: "fn process(input: &str) -> Result<()>".to_string(),
            line: 150,
            children: vec![],
        }];
        let result = format_outline(&items, 1, 100, 200).unwrap();
        assert!(result.contains("Outline of lines 101-200"));
        assert!(result.contains("L150"));
        assert!(result.contains("fn process"));
    }

    #[test]
    fn test_format_outline_before_and_after() {
        let items = vec![
            OutlineItem {
                kind: "struct",
                name: "Config".to_string(),
                signature: "struct Config".to_string(),
                line: 5,
                children: vec![],
            },
            OutlineItem {
                kind: "fn",
                name: "main".to_string(),
                signature: "fn main()".to_string(),
                line: 250,
                children: vec![],
            },
        ];
        let result = format_outline(&items, 50, 200, 300).unwrap();
        assert!(result.contains("Outline of lines 1-49"));
        assert!(result.contains("struct Config"));
        assert!(result.contains("Outline of lines 201-300"));
        assert!(result.contains("fn main"));
    }

    #[test]
    fn test_format_outline_with_children() {
        let items = vec![OutlineItem {
            kind: "impl",
            name: "Config".to_string(),
            signature: "impl Config".to_string(),
            line: 150,
            children: vec![OutlineItem {
                kind: "fn",
                name: "load".to_string(),
                signature: "fn load(path: &str) -> Self".to_string(),
                line: 155,
                children: vec![],
            }],
        }];
        let result = format_outline(&items, 1, 100, 200).unwrap();
        assert!(result.contains("impl Config"));
        assert!(result.contains("  // L155: fn load"));
    }

    #[cfg(feature = "tree-sitter-outlines")]
    #[test]
    fn test_generate_outline_rust() {
        let source = r#"
use std::io;

struct Config {
    name: String,
}

impl Config {
    fn new(name: &str) -> Self {
        Config { name: name.to_string() }
    }

    fn validate(&self) -> bool {
        !self.name.is_empty()
    }
}

fn main() {
    let c = Config::new("test");
}

#[cfg(test)]
mod tests {
    use super::*;
}
"#;
        let items = generate_outline(source, "test.rs");
        assert!(!items.is_empty(), "Should produce outline items");

        // Should find struct, impl with methods, fn main, and mod tests
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Config"), "Should find struct Config");
        assert!(names.contains(&"main"), "Should find fn main");
        assert!(names.contains(&"tests"), "Should find mod tests");

        // impl should have children
        let impl_item = items.iter().find(|i| i.kind == "impl").unwrap();
        assert_eq!(impl_item.children.len(), 2, "impl should have 2 methods");
    }

    #[cfg(feature = "tree-sitter-outlines")]
    #[test]
    fn test_generate_outline_python() {
        let source = r#"
class MyClass:
    def __init__(self, name):
        self.name = name

    def greet(self):
        print(f"Hello, {self.name}")

def main():
    c = MyClass("test")
"#;
        let items = generate_outline(source, "test.py");
        assert!(!items.is_empty());
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"MyClass"));
        assert!(names.contains(&"main"));
    }

    #[cfg(feature = "tree-sitter-outlines")]
    #[test]
    fn test_generate_outline_typescript() {
        let source = r#"
interface Config {
    name: string;
}

class App {
    constructor(private config: Config) {}

    start(): void {
        console.log(this.config.name);
    }
}

export function main(): void {
    const app = new App({ name: "test" });
}
"#;
        let items = generate_outline(source, "test.ts");
        assert!(!items.is_empty());
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Config"));
        assert!(names.contains(&"App"));
        assert!(names.contains(&"main"));
    }

    #[cfg(feature = "tree-sitter-outlines")]
    #[test]
    fn test_generate_outline_unsupported_lang() {
        let items = generate_outline("some content", "file.go");
        assert!(items.is_empty());
    }
}
