//! A2UI Component Definitions
//!
//! Static catalog of components the agent may emit. Each component maps to a
//! React component in the UI renderer
//! and exposes a set of typed props.
//!
//! The prompt generator turns these definitions into textual signatures that
//! teach the LLM what JSON to produce.

/// One prop of a component.
pub struct PropDef {
    /// Prop name as it appears in the JSON `props` object.
    pub name: &'static str,
    /// Type annotation shown in the prompt signature.
    pub type_annotation: &'static str,
    /// Whether this prop is optional.
    pub optional: bool,
    /// Short prop description (optional; "" suppresses).
    pub description: &'static str,
}

/// One component available to the agent.
pub struct ComponentDef {
    /// Component name (PascalCase) used as the `type` field in JSON.
    pub name: &'static str,
    /// Ordered list of props.
    pub props: &'static [PropDef],
    /// Whether the component accepts a `children` array of nested nodes.
    pub has_children: bool,
    /// Human-readable description.
    pub description: &'static str,
}

/// All components in the default A2UI catalog.
///
/// Keep this catalog small and cross-platform friendly. Teams can add their own
/// components in a forked catalog per spec v0.9's prompt-first design.
#[allow(clippy::vec_init_then_push)]
pub fn all_components() -> Vec<ComponentDef> {
    let mut v = Vec::new();

    // ---------- Layout ----------
    v.push(ComponentDef {
        name: "Stack",
        props: &[
            PropDef {
                name: "direction",
                type_annotation: "\"row\" | \"column\"",
                optional: true,
                description: "default column",
            },
            PropDef {
                name: "gap",
                type_annotation: "\"none\" | \"sm\" | \"md\" | \"lg\"",
                optional: true,
                description: "default md",
            },
            PropDef {
                name: "align",
                type_annotation: "\"start\" | \"center\" | \"end\" | \"stretch\"",
                optional: true,
                description: "",
            },
        ],
        has_children: true,
        description: "Flex container for stacking children vertically or horizontally.",
    });

    v.push(ComponentDef {
        name: "Card",
        props: &[PropDef {
            name: "variant",
            type_annotation: "\"default\" | \"muted\"",
            optional: true,
            description: "",
        }],
        has_children: true,
        description: "Bordered container with padding. Preferred root for most responses.",
    });

    v.push(ComponentDef {
        name: "Separator",
        props: &[],
        has_children: false,
        description: "Horizontal divider line.",
    });

    // ---------- Content ----------
    v.push(ComponentDef {
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
                type_annotation: "1 | 2 | 3 | 4",
                optional: true,
                description: "default 3",
            },
        ],
        has_children: false,
        description: "Section heading.",
    });

    v.push(ComponentDef {
        name: "Text",
        props: &[
            PropDef {
                name: "text",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "tone",
                type_annotation: "\"default\" | \"muted\"",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Paragraph of text.",
    });

    v.push(ComponentDef {
        name: "Callout",
        props: &[
            PropDef {
                name: "text",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "variant",
                type_annotation: "\"info\" | \"success\" | \"warning\" | \"error\"",
                optional: true,
                description: "default info",
            },
            PropDef {
                name: "title",
                type_annotation: "string",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Highlighted notice block.",
    });

    v.push(ComponentDef {
        name: "Badge",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "variant",
                type_annotation: "\"default\" | \"success\" | \"warning\" | \"error\"",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Compact tag/badge.",
    });

    v.push(ComponentDef {
        name: "Image",
        props: &[
            PropDef {
                name: "src",
                type_annotation: "string",
                optional: false,
                description: "URL",
            },
            PropDef {
                name: "alt",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "caption",
                type_annotation: "string",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Image with optional caption.",
    });

    v.push(ComponentDef {
        name: "CodeBlock",
        props: &[
            PropDef {
                name: "code",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "language",
                type_annotation: "string",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Monospaced code block.",
    });

    // ---------- Data ----------
    v.push(ComponentDef {
        name: "List",
        props: &[PropDef {
            name: "ordered",
            type_annotation: "boolean",
            optional: true,
            description: "default false",
        }],
        has_children: true,
        description: "List of ListItem children.",
    });

    v.push(ComponentDef {
        name: "ListItem",
        props: &[
            PropDef {
                name: "title",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "value",
                type_annotation: "string",
                optional: true,
                description: "Secondary right-aligned text.",
            },
            PropDef {
                name: "description",
                type_annotation: "string",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "One list entry.",
    });

    v.push(ComponentDef {
        name: "Table",
        props: &[
            PropDef {
                name: "columns",
                type_annotation: "string[]",
                optional: false,
                description: "Column header labels.",
            },
            PropDef {
                name: "rows",
                type_annotation: "(string | number | boolean)[][]",
                optional: false,
                description: "Row-major cell data.",
            },
        ],
        has_children: false,
        description: "Tabular data.",
    });

    // ---------- Forms ----------
    v.push(ComponentDef {
        name: "Form",
        props: &[
            PropDef {
                name: "name",
                type_annotation: "string",
                optional: false,
                description: "Form identifier.",
            },
            PropDef {
                name: "submitLabel",
                type_annotation: "string",
                optional: true,
                description: "default \"Submit\"",
            },
            PropDef {
                name: "submitMessage",
                type_annotation: "string",
                optional: true,
                description: "Chat message to send on submit. Default: \"Submitted {name} form\".",
            },
        ],
        has_children: true,
        description: "Form wrapping field children and a submit button. Submission sends a chat message summarizing field values.",
    });

    v.push(ComponentDef {
        name: "TextField",
        props: &[
            PropDef {
                name: "name",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "placeholder",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "required",
                type_annotation: "boolean",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Single-line text input.",
    });

    v.push(ComponentDef {
        name: "Textarea",
        props: &[
            PropDef {
                name: "name",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "placeholder",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "rows",
                type_annotation: "number",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Multi-line text input.",
    });

    v.push(ComponentDef {
        name: "Select",
        props: &[
            PropDef {
                name: "name",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: true,
                description: "",
            },
            PropDef {
                name: "options",
                type_annotation: "{ label: string, value: string }[]",
                optional: false,
                description: "",
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Dropdown select.",
    });

    v.push(ComponentDef {
        name: "Checkbox",
        props: &[
            PropDef {
                name: "name",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "defaultChecked",
                type_annotation: "boolean",
                optional: true,
                description: "",
            },
        ],
        has_children: false,
        description: "Single checkbox. Submitted as boolean.",
    });

    // ---------- Actions ----------
    v.push(ComponentDef {
        name: "Button",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
                description: "",
            },
            PropDef {
                name: "action",
                type_annotation: "Action",
                optional: true,
                description: "See Actions section. Omit for non-interactive buttons.",
            },
            PropDef {
                name: "variant",
                type_annotation: "\"primary\" | \"secondary\" | \"ghost\" | \"destructive\"",
                optional: true,
                description: "default primary",
            },
        ],
        has_children: false,
        description: "Clickable button with optional action.",
    });

    v.push(ComponentDef {
        name: "ButtonGroup",
        props: &[PropDef {
            name: "align",
            type_annotation: "\"start\" | \"center\" | \"end\"",
            optional: true,
            description: "default start",
        }],
        has_children: true,
        description: "Row of Button children.",
    });

    v
}
