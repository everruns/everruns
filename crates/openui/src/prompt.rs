//! OpenUI System Prompt Generator
//!
//! Generates the system prompt that instructs LLMs to respond in OpenUI Lang.
//! This is a Rust reimplementation of the TypeScript prompt generator.
//!
//! Ref: packages/react-lang/src/parser/prompt.ts

use crate::components::{ComponentDef, Library};

/// Options for customizing the generated prompt.
///
/// Ref: packages/react-lang/src/library.ts `PromptOptions` interface
#[derive(Default)]
pub struct PromptOptions {
    /// Custom preamble text (replaces the default)
    pub preamble: Option<String>,
    /// Additional rules appended to the prompt
    pub additional_rules: Vec<String>,
    /// Example OpenUI Lang code blocks
    pub examples: Vec<String>,
}

/// Default preamble for the system prompt.
///
/// Modified from the upstream version to instruct wrapping in ```openui code blocks.
///
/// Ref: packages/react-lang/src/parser/prompt.ts `PREAMBLE`
const PREAMBLE: &str = "You are an AI assistant that can generate rich interactive UI using openui-lang, a declarative UI language. When the user asks for visual content (dashboards, charts, tables, forms, lists, etc.), respond with openui-lang code wrapped in a ```openui fenced code block. You may include explanatory markdown text before and/or after the code block.";

/// Generate the syntax rules section.
///
/// Ref: packages/react-lang/src/parser/prompt.ts `syntaxRules()`
fn syntax_rules(root_name: &str) -> String {
    format!(
        r#"## Syntax Rules
1. Each statement is on its own line: `identifier = Expression`
2. `root` is the entry point — every program must define `root = {root_name}(...)`
3. Expressions are: strings ("..."), numbers, booleans (true/false), arrays ([...]), objects ({{...}}), or component calls TypeName(arg1, arg2, ...)
4. Use references for readability: define `name = ...` on one line, then use `name` later
5. EVERY variable (except root) MUST be referenced by at least one other variable. Unreferenced variables are silently dropped and will NOT render.
6. Arguments are POSITIONAL (order matters, not names)
7. Optional arguments can be omitted from the end
8. No operators, no logic, no variables — only declarations
9. Strings use double quotes with backslash escaping"#
    )
}

/// Generate the streaming/hoisting rules section.
///
/// Ref: packages/react-lang/src/parser/prompt.ts `streamingRules()`
fn streaming_rules(root_name: &str) -> String {
    format!(
        r#"## Hoisting & Streaming (CRITICAL)
openui-lang supports hoisting: a reference can be used BEFORE it is defined. The parser resolves all references after the full input is parsed.
During streaming, the output is re-parsed on every chunk. Undefined references are temporarily unresolved and appear once their definitions stream in.
**Recommended statement order for optimal streaming:**
1. `root = {root_name}(...)` — UI shell appears immediately
2. Component definitions — fill in as they stream
3. Data values — leaf content last
Always write the root = {root_name}(...) statement first so the UI shell appears immediately."#
    )
}

/// Generate the important rules section.
///
/// Ref: packages/react-lang/src/parser/prompt.ts `importantRules()`
fn important_rules(root_name: &str) -> String {
    format!(
        r#"## Important Rules
- ALWAYS start with root = {root_name}(...)
- Write statements in TOP-DOWN order: root → components → data (leverages hoisting for progressive streaming)
- Each statement on its own line
- When asked about data, generate realistic/plausible data
- Choose components that best represent the content (tables for comparisons, charts for trends, forms for input, etc.)
- NEVER define a variable without referencing it from the tree. Every variable must be reachable from root.
- Wrap your openui-lang code in a ```openui fenced code block
- You may include explanatory text before or after the code block"#
    )
}

/// Build the signature line for a single component.
///
/// Produces output like: `Stack(children: any[], direction?: "row" | "column", ...) — Flex container.`
///
/// Ref: packages/react-lang/src/parser/prompt.ts `buildComponentLine()`
fn build_component_line(component: &ComponentDef) -> String {
    let params: Vec<String> = component
        .props
        .iter()
        .map(|p| {
            if p.optional {
                format!("{}?: {}", p.name, p.type_annotation)
            } else {
                format!("{}: {}", p.name, p.type_annotation)
            }
        })
        .collect();

    let sig = format!("{}({})", component.name, params.join(", "));

    if component.description.is_empty() {
        sig
    } else {
        format!("{} — {}", sig, component.description)
    }
}

/// Generate the component signatures section.
///
/// Ref: packages/react-lang/src/parser/prompt.ts `generateComponentSignatures()`
fn generate_component_signatures(library: &Library) -> String {
    let mut lines = vec![
        "## Component Signatures".to_string(),
        String::new(),
        "Arguments marked with ? are optional. Sub-components can be inline or referenced; prefer references for better streaming.".to_string(),
    ];

    if library.groups.is_empty() {
        // Ungrouped — list all components
        lines.push(String::new());
        for comp in &library.components {
            lines.push(build_component_line(comp));
        }
    } else {
        // Build a lookup map for components by name
        let comp_map: std::collections::HashMap<&str, &ComponentDef> =
            library.components.iter().map(|c| (c.name, c)).collect();

        let mut grouped_names = std::collections::HashSet::new();

        for group in &library.groups {
            lines.push(String::new());
            lines.push(format!("### {}", group.name));

            for name in &group.components {
                if grouped_names.contains(name) {
                    continue;
                }
                if let Some(comp) = comp_map.get(name) {
                    grouped_names.insert(*name);
                    lines.push(build_component_line(comp));
                }
            }

            for note in &group.notes {
                lines.push(note.to_string());
            }
        }

        // Any ungrouped components
        let ungrouped: Vec<&ComponentDef> = library
            .components
            .iter()
            .filter(|c| !grouped_names.contains(c.name))
            .collect();

        if !ungrouped.is_empty() {
            lines.push(String::new());
            lines.push("### Ungrouped".to_string());
            for comp in ungrouped {
                lines.push(build_component_line(comp));
            }
        }
    }

    lines.join("\n")
}

/// Generate the complete OpenUI system prompt.
///
/// This is the main entry point. Produces a prompt that, when prepended to the
/// system prompt, instructs the LLM to respond with OpenUI Lang code.
///
/// Ref: packages/react-lang/src/parser/prompt.ts `generatePrompt()`
#[allow(clippy::vec_init_then_push)]
pub fn generate_prompt(library: &Library, options: &PromptOptions) -> String {
    let root_name = library.root;
    let mut parts = Vec::new();

    // 1. Preamble
    parts.push(options.preamble.as_deref().unwrap_or(PREAMBLE).to_string());
    parts.push(String::new());

    // 2. Syntax rules
    parts.push(syntax_rules(root_name));
    parts.push(String::new());

    // 3. Component signatures
    parts.push(generate_component_signatures(library));
    parts.push(String::new());

    // 4. Streaming/hoisting rules
    parts.push(streaming_rules(root_name));

    // 5. Optional examples
    if !options.examples.is_empty() {
        parts.push(String::new());
        parts.push("## Examples".to_string());
        parts.push(String::new());
        for ex in &options.examples {
            parts.push(ex.clone());
            parts.push(String::new());
        }
    }

    // 6. Important rules
    parts.push(important_rules(root_name));

    // 7. Additional rules
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
    use super::*;
    use crate::components::PropDef;

    fn test_library() -> Library {
        Library {
            root: "Stack",
            components: vec![
                ComponentDef {
                    name: "Stack",
                    props: &[PropDef {
                        name: "children",
                        type_annotation: "any[]",
                        optional: false,
                    }],
                    description: "Flex container",
                },
                ComponentDef {
                    name: "TextContent",
                    props: &[
                        PropDef {
                            name: "text",
                            type_annotation: "string",
                            optional: false,
                        },
                        PropDef {
                            name: "size",
                            type_annotation: "\"small\" | \"large\"",
                            optional: true,
                        },
                    ],
                    description: "Text block",
                },
            ],
            groups: vec![],
        }
    }

    #[test]
    fn test_generate_prompt_contains_preamble() {
        let lib = test_library();
        let prompt = generate_prompt(&lib, &PromptOptions::default());
        assert!(prompt.contains("openui-lang"));
        assert!(prompt.contains("```openui"));
    }

    #[test]
    fn test_generate_prompt_contains_syntax_rules() {
        let lib = test_library();
        let prompt = generate_prompt(&lib, &PromptOptions::default());
        assert!(prompt.contains("## Syntax Rules"));
        assert!(prompt.contains("root = Stack(...)"));
        assert!(prompt.contains("POSITIONAL"));
    }

    #[test]
    fn test_generate_prompt_contains_component_signatures() {
        let lib = test_library();
        let prompt = generate_prompt(&lib, &PromptOptions::default());
        assert!(prompt.contains("## Component Signatures"));
        assert!(prompt.contains("Stack(children: any[]) — Flex container"));
        assert!(
            prompt.contains("TextContent(text: string, size?: \"small\" | \"large\") — Text block")
        );
    }

    #[test]
    fn test_generate_prompt_contains_streaming_rules() {
        let lib = test_library();
        let prompt = generate_prompt(&lib, &PromptOptions::default());
        assert!(prompt.contains("## Hoisting & Streaming"));
        assert!(prompt.contains("hoisting"));
    }

    #[test]
    fn test_generate_prompt_contains_important_rules() {
        let lib = test_library();
        let prompt = generate_prompt(&lib, &PromptOptions::default());
        assert!(prompt.contains("## Important Rules"));
        assert!(prompt.contains("ALWAYS start with root = Stack(...)"));
    }

    #[test]
    fn test_custom_preamble() {
        let lib = test_library();
        let options = PromptOptions {
            preamble: Some("Custom preamble here".to_string()),
            ..Default::default()
        };
        let prompt = generate_prompt(&lib, &options);
        assert!(prompt.contains("Custom preamble here"));
        assert!(!prompt.contains("You are an AI assistant"));
    }

    #[test]
    fn test_additional_rules() {
        let lib = test_library();
        let options = PromptOptions {
            additional_rules: vec!["Always use dark theme".to_string()],
            ..Default::default()
        };
        let prompt = generate_prompt(&lib, &options);
        assert!(prompt.contains("- Always use dark theme"));
    }

    #[test]
    fn test_examples() {
        let lib = test_library();
        let options = PromptOptions {
            examples: vec!["root = Stack([text])\ntext = TextContent(\"Hello\")".to_string()],
            ..Default::default()
        };
        let prompt = generate_prompt(&lib, &options);
        assert!(prompt.contains("## Examples"));
        assert!(prompt.contains("root = Stack([text])"));
    }

    #[test]
    fn test_grouped_components() {
        use crate::groups::ComponentGroup;

        let lib = Library {
            root: "Stack",
            components: vec![
                ComponentDef {
                    name: "Stack",
                    props: &[PropDef {
                        name: "children",
                        type_annotation: "any[]",
                        optional: false,
                    }],
                    description: "Flex container",
                },
                ComponentDef {
                    name: "TextContent",
                    props: &[PropDef {
                        name: "text",
                        type_annotation: "string",
                        optional: false,
                    }],
                    description: "Text block",
                },
            ],
            groups: vec![
                ComponentGroup {
                    name: "Layout",
                    components: vec!["Stack"],
                    notes: vec![],
                },
                ComponentGroup {
                    name: "Content",
                    components: vec!["TextContent"],
                    notes: vec!["Use TextContent for plain text."],
                },
            ],
        };

        let prompt = generate_prompt(&lib, &PromptOptions::default());
        assert!(prompt.contains("### Layout"));
        assert!(prompt.contains("### Content"));
        assert!(prompt.contains("Use TextContent for plain text."));
    }

    #[test]
    fn test_build_component_line_required_only() {
        let comp = ComponentDef {
            name: "Table",
            props: &[PropDef {
                name: "data",
                type_annotation: "any[]",
                optional: false,
            }],
            description: "Data table",
        };
        let line = build_component_line(&comp);
        assert_eq!(line, "Table(data: any[]) — Data table");
    }

    #[test]
    fn test_build_component_line_mixed_props() {
        let comp = ComponentDef {
            name: "Button",
            props: &[
                PropDef {
                    name: "label",
                    type_annotation: "string",
                    optional: false,
                },
                PropDef {
                    name: "variant",
                    type_annotation: "\"primary\" | \"secondary\"",
                    optional: true,
                },
            ],
            description: "Clickable button",
        };
        let line = build_component_line(&comp);
        assert_eq!(
            line,
            "Button(label: string, variant?: \"primary\" | \"secondary\") — Clickable button"
        );
    }

    #[test]
    fn test_build_component_line_no_props() {
        let comp = ComponentDef {
            name: "Separator",
            props: &[],
            description: "Visual divider",
        };
        let line = build_component_line(&comp);
        assert_eq!(line, "Separator() — Visual divider");
    }
}
