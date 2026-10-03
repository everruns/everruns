//! A2UI System Prompt Generator
//!
//! Generates the prompt that teaches the LLM how to emit A2UI JSON component
//! trees inside ```a2ui fenced code blocks.

use std::collections::HashMap;

use super::catalog::Catalog;
use super::components::ComponentDef;

/// Options for customizing the generated prompt.
#[derive(Default)]
pub struct PromptOptions {
    /// Replaces the default preamble text.
    pub preamble: Option<String>,
    /// Extra rules appended after the standard rules section.
    pub additional_rules: Vec<String>,
    /// Example ```a2ui blocks rendered into an Examples section.
    pub examples: Vec<String>,
}

const PREAMBLE: &str = "You are an AI assistant with access to A2UI, an open generative-UI protocol. \
When the user asks for visual content (summaries, dashboards, lists, forms, action buttons, etc.), \
respond with a declarative UI tree as JSON wrapped in a ```a2ui fenced code block. \
You may include plain markdown before and/or after the block.";

fn schema_rules() -> &'static str {
    r#"## Schema
Every node is a JSON object with this shape:
```
{ "type": "ComponentName", "props": { ... }, "children": [ ... ] }
```
- `type` (required): one of the component names below. Emit `type` verbatim (PascalCase).
- `props` (optional): object whose keys must match the component's declared props. Omit props you do not need.
- `children` (optional): array of nested nodes. Only emit `children` on components that declare it.
- Unknown components, props, or enum values are ignored by the renderer. Stay within the catalog."#
}

fn actions_section() -> &'static str {
    r#"## Actions
Some props (e.g. `Button.action`) accept an Action object. Supported actions:
- `{ "type": "message", "text": string }` — sends `text` as a new chat message from the user.
- `{ "type": "open_url", "url": string }` — opens `url` in a new tab.
Omit the `action` prop for decorative buttons."#
}

fn streaming_rules(root_hint: &str) -> String {
    format!(
        r#"## Streaming
Output is re-parsed as it streams. To render the shell fast:
1. Emit the outer `{root_hint}` object first.
2. Emit top-level children before their deep content.
3. Emit leaf `props` like long strings last.
Partial trees render progressively; keep the JSON syntactically valid as you go."#
    )
}

fn important_rules(root_hint: &str) -> String {
    format!(
        r#"## Rules
- Start with a single top-level node (usually `{root_hint}`). Do not wrap it in an array.
- Only use components and props listed in the catalog.
- Prefer `List`/`Table` over repeated handcrafted rows.
- Keep trees small and focused — one A2UI block per idea.
- Use action buttons to suggest follow-up questions or URLs, not to execute backend work.
- You may mix markdown text before and after the block. Do NOT put markdown inside the JSON.
- Always close the ```a2ui fence with ``` on its own line."#
    )
}

fn build_component_line(component: &ComponentDef) -> String {
    let mut params: Vec<String> = component
        .props
        .iter()
        .map(|p| {
            let head = if p.optional {
                format!("{}?: {}", p.name, p.type_annotation)
            } else {
                format!("{}: {}", p.name, p.type_annotation)
            };
            if p.description.is_empty() {
                head
            } else {
                format!("{head} ({})", p.description)
            }
        })
        .collect();

    if component.has_children {
        params.push("children?: Node[]".to_string());
    }

    let sig = format!("{}({})", component.name, params.join(", "));

    if component.description.is_empty() {
        sig
    } else {
        format!("{sig} — {}", component.description)
    }
}

fn generate_catalog_section(catalog: &Catalog) -> String {
    let mut lines = vec![
        "## Catalog".to_string(),
        String::new(),
        "Each signature shows the component's props and whether it accepts children. \
         `?` marks optional props."
            .to_string(),
    ];

    let comp_map: HashMap<&str, &ComponentDef> =
        catalog.components.iter().map(|c| (c.name, c)).collect();

    let mut covered = std::collections::HashSet::new();

    for cat in &catalog.categories {
        lines.push(String::new());
        lines.push(format!("### {}", cat.name));
        for name in &cat.components {
            if covered.contains(name) {
                continue;
            }
            if let Some(comp) = comp_map.get(name) {
                covered.insert(*name);
                lines.push(build_component_line(comp));
            }
        }
        for note in &cat.notes {
            lines.push(note.to_string());
        }
    }

    // Ungrouped components (shouldn't happen, but included for robustness).
    let ungrouped: Vec<&ComponentDef> = catalog
        .components
        .iter()
        .filter(|c| !covered.contains(c.name))
        .collect();
    if !ungrouped.is_empty() {
        lines.push(String::new());
        lines.push("### Other".to_string());
        for comp in ungrouped {
            lines.push(build_component_line(comp));
        }
    }

    lines.join("\n")
}

/// Build the full A2UI system prompt.
#[allow(clippy::vec_init_then_push)]
pub fn generate_prompt(catalog: &Catalog, options: &PromptOptions) -> String {
    let mut parts = Vec::new();

    parts.push(options.preamble.as_deref().unwrap_or(PREAMBLE).to_string());
    parts.push(String::new());

    parts.push(schema_rules().to_string());
    parts.push(String::new());

    parts.push(generate_catalog_section(catalog));
    parts.push(String::new());

    parts.push(actions_section().to_string());
    parts.push(String::new());

    parts.push(streaming_rules(catalog.root_hint));
    parts.push(String::new());

    parts.push(important_rules(catalog.root_hint));

    if !options.examples.is_empty() {
        parts.push(String::new());
        parts.push("## Examples".to_string());
        for ex in &options.examples {
            parts.push(String::new());
            parts.push(ex.clone());
        }
    }

    if !options.additional_rules.is_empty() {
        parts.push(String::new());
        for rule in &options.additional_rules {
            parts.push(format!("- {rule}"));
        }
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::catalog::ComponentCategory;
    use super::super::components::PropDef;
    use super::*;

    fn fixture() -> Catalog {
        Catalog {
            root_hint: "Card",
            components: vec![
                ComponentDef {
                    name: "Card",
                    props: &[],
                    has_children: true,
                    description: "Bordered container.",
                },
                ComponentDef {
                    name: "Heading",
                    props: &[
                        PropDef {
                            name: "text",
                            type_annotation: "string",
                            optional: false,
                            description: "",
                        },
                        PropDef {
                            name: "level",
                            type_annotation: "1 | 2 | 3",
                            optional: true,
                            description: "default 3",
                        },
                    ],
                    has_children: false,
                    description: "Section heading.",
                },
            ],
            categories: vec![ComponentCategory {
                name: "Layout",
                components: vec!["Card", "Heading"],
                notes: vec!["Prefer Card as the root."],
            }],
        }
    }

    #[test]
    fn prompt_contains_preamble_and_fence() {
        let p = generate_prompt(&fixture(), &PromptOptions::default());
        assert!(p.contains("A2UI"));
        assert!(p.contains("```a2ui"));
    }

    #[test]
    fn prompt_contains_schema() {
        let p = generate_prompt(&fixture(), &PromptOptions::default());
        assert!(p.contains("## Schema"));
        assert!(p.contains("\"type\""));
        assert!(p.contains("\"children\""));
    }

    #[test]
    fn prompt_contains_actions() {
        let p = generate_prompt(&fixture(), &PromptOptions::default());
        assert!(p.contains("## Actions"));
        assert!(p.contains("\"message\""));
        assert!(p.contains("\"open_url\""));
    }

    #[test]
    fn component_line_optional_prop() {
        let comp = ComponentDef {
            name: "Heading",
            props: &[
                PropDef {
                    name: "text",
                    type_annotation: "string",
                    optional: false,
                    description: "",
                },
                PropDef {
                    name: "level",
                    type_annotation: "1 | 2 | 3",
                    optional: true,
                    description: "",
                },
            ],
            has_children: false,
            description: "Section heading.",
        };
        let line = build_component_line(&comp);
        assert_eq!(
            line,
            "Heading(text: string, level?: 1 | 2 | 3) — Section heading."
        );
    }

    #[test]
    fn component_line_with_children() {
        let comp = ComponentDef {
            name: "Card",
            props: &[],
            has_children: true,
            description: "Container.",
        };
        let line = build_component_line(&comp);
        assert_eq!(line, "Card(children?: Node[]) — Container.");
    }

    #[test]
    fn category_notes_render() {
        let p = generate_prompt(&fixture(), &PromptOptions::default());
        assert!(p.contains("Prefer Card as the root."));
    }

    #[test]
    fn custom_preamble_replaces_default() {
        let opts = PromptOptions {
            preamble: Some("CUSTOM PREAMBLE".to_string()),
            ..Default::default()
        };
        let p = generate_prompt(&fixture(), &opts);
        assert!(p.starts_with("CUSTOM PREAMBLE"));
    }

    #[test]
    fn examples_section_included() {
        let opts = PromptOptions {
            examples: vec![
                r#"```a2ui
{"type":"Card"}
```"#
                    .to_string(),
            ],
            ..Default::default()
        };
        let p = generate_prompt(&fixture(), &opts);
        assert!(p.contains("## Examples"));
        assert!(p.contains("{\"type\":\"Card\"}"));
    }

    #[test]
    fn additional_rules_render_as_bullets() {
        let opts = PromptOptions {
            additional_rules: vec!["Stay concise".to_string()],
            ..Default::default()
        };
        let p = generate_prompt(&fixture(), &opts);
        assert!(p.contains("- Stay concise"));
    }
}
