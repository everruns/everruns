//! OpenUI Component Definitions
//!
//! Static definitions of all OpenUI components, their props, and descriptions.
//! These mirror the TypeScript `defineComponent()` calls in `@openuidev/react-ui`.
//!
//! Ref: packages/react-ui/src/genui-lib/ (each component directory)
//! Ref: packages/react-ui/src/genui-lib/openuiChatLibrary.tsx (chat library)
//! Ref: packages/react-ui/src/genui-lib/openuiLibrary.tsx (base library)

use crate::groups::ComponentGroup;

/// A single prop/argument definition for a component.
/// Maps to a key in the TypeScript Zod schema's `.shape`.
///
/// Ref: packages/react-lang/src/parser/prompt.ts `analyzeFields()`
pub struct PropDef {
    /// Prop name (matches Zod schema key name, used as positional arg name)
    pub name: &'static str,
    /// Type annotation string shown in the prompt signature.
    /// Matches what the TS `resolveTypeAnnotation()` would produce.
    pub type_annotation: &'static str,
    /// Whether this prop is optional (z.optional() in Zod)
    pub optional: bool,
}

/// A component definition.
/// Equivalent to TS `DefinedComponent` from `defineComponent()`.
///
/// Ref: packages/react-lang/src/library.ts `defineComponent()`
pub struct ComponentDef {
    /// Component name (PascalCase, used in OpenUI Lang as `TypeName(...)`)
    pub name: &'static str,
    /// Ordered list of props (order defines positional argument mapping)
    pub props: &'static [PropDef],
    /// Human-readable description shown in the prompt
    pub description: &'static str,
}

/// The full component library definition.
///
/// Ref: packages/react-lang/src/library.ts `Library` interface
pub struct Library {
    /// Root component name (entry point for `root = Root(...)`)
    pub root: &'static str,
    /// All component definitions
    pub components: Vec<ComponentDef>,
    /// Component groups for organized prompt output
    pub groups: Vec<ComponentGroup>,
}

/// Returns all component definitions for the chat library.
///
/// Ref: packages/react-ui/src/genui-lib/openuiChatLibrary.tsx
#[allow(clippy::vec_init_then_push)]
pub fn all_components() -> Vec<ComponentDef> {
    let mut components = Vec::new();

    // =========================================================================
    // Layout components
    // Ref: packages/react-ui/src/genui-lib/Stack/
    // =========================================================================

    // Ref: packages/react-ui/src/genui-lib/Stack/schema.ts — StackSchema
    components.push(ComponentDef {
        name: "Stack",
        props: &[
            PropDef { name: "children", type_annotation: "any[]", optional: false },
            PropDef { name: "direction", type_annotation: "\"row\" | \"column\"", optional: true },
            PropDef { name: "gap", type_annotation: "\"none\" | \"xs\" | \"s\" | \"m\" | \"l\" | \"xl\" | \"2xl\"", optional: true },
            PropDef { name: "align", type_annotation: "\"start\" | \"center\" | \"end\" | \"stretch\" | \"baseline\"", optional: true },
            PropDef { name: "justify", type_annotation: "\"start\" | \"center\" | \"end\" | \"between\" | \"around\" | \"evenly\"", optional: true },
            PropDef { name: "wrap", type_annotation: "boolean", optional: true },
        ],
        description: "Flex container. direction: \"row\"|\"column\" (default \"column\"). gap: \"none\"|\"xs\"|\"s\"|\"m\"|\"l\"|\"xl\"|\"2xl\" (default \"m\"). align: \"start\"|\"center\"|\"end\"|\"stretch\"|\"baseline\". justify: \"start\"|\"center\"|\"end\"|\"between\"|\"around\"|\"evenly\".",
    });

    // Ref: packages/react-ui/src/genui-lib/Tabs/
    components.push(ComponentDef {
        name: "Tabs",
        props: &[PropDef {
            name: "tabs",
            type_annotation: "TabItem[]",
            optional: false,
        }],
        description: "Tabbed container with switchable panels",
    });

    // Ref: packages/react-ui/src/genui-lib/Tabs/ (TabItem sub-component)
    components.push(ComponentDef {
        name: "TabItem",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "children",
                type_annotation: "any[]",
                optional: false,
            },
        ],
        description: "One tab with label and content",
    });

    // Ref: packages/react-ui/src/genui-lib/Accordion/
    components.push(ComponentDef {
        name: "Accordion",
        props: &[PropDef {
            name: "items",
            type_annotation: "AccordionItem[]",
            optional: false,
        }],
        description: "Collapsible sections",
    });

    components.push(ComponentDef {
        name: "AccordionItem",
        props: &[
            PropDef {
                name: "title",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "children",
                type_annotation: "any[]",
                optional: false,
            },
        ],
        description: "One accordion section with title and content",
    });

    // Ref: packages/react-ui/src/genui-lib/Steps/
    components.push(ComponentDef {
        name: "Steps",
        props: &[PropDef {
            name: "steps",
            type_annotation: "StepsItem[]",
            optional: false,
        }],
        description: "Step-by-step progress indicator with content",
    });

    components.push(ComponentDef {
        name: "StepsItem",
        props: &[
            PropDef {
                name: "title",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "children",
                type_annotation: "any[]",
                optional: false,
            },
        ],
        description: "One step with title and content",
    });

    // Ref: packages/react-ui/src/genui-lib/Carousel/
    components.push(ComponentDef {
        name: "Carousel",
        props: &[PropDef {
            name: "items",
            type_annotation: "Card[]",
            optional: false,
        }],
        description: "Horizontally scrollable card carousel",
    });

    // Ref: packages/react-ui/src/genui-lib/Separator/
    components.push(ComponentDef {
        name: "Separator",
        props: &[],
        description: "Visual divider line",
    });

    // =========================================================================
    // Content components
    // Ref: packages/react-ui/src/genui-lib/{Card,CardHeader,TextContent,...}/
    // =========================================================================

    // Ref: packages/react-ui/src/genui-lib/Card/schema.ts — CardSchema
    components.push(ComponentDef {
        name: "Card",
        props: &[
            PropDef { name: "children", type_annotation: "any[]", optional: false },
            PropDef { name: "variant", type_annotation: "\"card\" | \"sunk\" | \"clear\"", optional: true },
            PropDef { name: "direction", type_annotation: "\"row\" | \"column\"", optional: true },
            PropDef { name: "gap", type_annotation: "\"none\" | \"xs\" | \"s\" | \"m\" | \"l\" | \"xl\" | \"2xl\"", optional: true },
            PropDef { name: "align", type_annotation: "\"start\" | \"center\" | \"end\" | \"stretch\" | \"baseline\"", optional: true },
            PropDef { name: "justify", type_annotation: "\"start\" | \"center\" | \"end\" | \"between\" | \"around\" | \"evenly\"", optional: true },
            PropDef { name: "wrap", type_annotation: "boolean", optional: true },
        ],
        description: "Styled container. variant: card (elevated) | sunk (recessed) | clear (transparent). Always full width. Accepts all Stack flex params (default: direction column). Cards flex to share space in row/wrap layouts.",
    });

    // Ref: packages/react-ui/src/genui-lib/CardHeader/
    components.push(ComponentDef {
        name: "CardHeader",
        props: &[
            PropDef {
                name: "title",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "subtitle",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Card title and optional subtitle",
    });

    // Ref: packages/react-ui/src/genui-lib/TextContent/schema.ts
    components.push(ComponentDef {
        name: "TextContent",
        props: &[
            PropDef { name: "text", type_annotation: "string", optional: false },
            PropDef { name: "size", type_annotation: "\"small\" | \"default\" | \"large\" | \"small-heavy\" | \"large-heavy\"", optional: true },
        ],
        description: "Text block. Supports markdown. Optional size: \"small\" | \"default\" | \"large\" | \"small-heavy\" | \"large-heavy\".",
    });

    // Ref: packages/react-ui/src/genui-lib/MarkDownRenderer/
    components.push(ComponentDef {
        name: "MarkDownRenderer",
        props: &[PropDef {
            name: "text",
            type_annotation: "string",
            optional: false,
        }],
        description: "Renders markdown text",
    });

    // Ref: packages/react-ui/src/genui-lib/Callout/
    components.push(ComponentDef {
        name: "Callout",
        props: &[
            PropDef {
                name: "title",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "children",
                type_annotation: "any[]",
                optional: false,
            },
            PropDef {
                name: "variant",
                type_annotation: "\"info\" | \"warning\" | \"error\" | \"success\"",
                optional: true,
            },
        ],
        description: "Highlighted block with title, content, and variant",
    });

    // Ref: packages/react-ui/src/genui-lib/TextCallout/
    components.push(ComponentDef {
        name: "TextCallout",
        props: &[
            PropDef {
                name: "text",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "variant",
                type_annotation: "\"info\" | \"warning\" | \"error\" | \"success\"",
                optional: true,
            },
        ],
        description: "Simple text callout with variant",
    });

    // Ref: packages/react-ui/src/genui-lib/CodeBlock/
    components.push(ComponentDef {
        name: "CodeBlock",
        props: &[
            PropDef {
                name: "code",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "language",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Syntax-highlighted code block",
    });

    // Ref: packages/react-ui/src/genui-lib/Image/
    components.push(ComponentDef {
        name: "Image",
        props: &[
            PropDef {
                name: "src",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "alt",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Single image",
    });

    // Ref: packages/react-ui/src/genui-lib/ImageBlock/
    components.push(ComponentDef {
        name: "ImageBlock",
        props: &[
            PropDef {
                name: "src",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "alt",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "caption",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Image with optional caption",
    });

    // Ref: packages/react-ui/src/genui-lib/ImageGallery/
    components.push(ComponentDef {
        name: "ImageGallery",
        props: &[PropDef {
            name: "images",
            type_annotation: "Image[]",
            optional: false,
        }],
        description: "Grid of images",
    });

    // =========================================================================
    // Tables
    // Ref: packages/react-ui/src/genui-lib/Table/
    // =========================================================================

    // Ref: packages/react-ui/src/genui-lib/Table/index.tsx — Table schema inline
    components.push(ComponentDef {
        name: "Table",
        props: &[
            PropDef {
                name: "columns",
                type_annotation: "Col[]",
                optional: false,
            },
            PropDef {
                name: "rows",
                type_annotation: "(string | number | boolean)[][]",
                optional: false,
            },
        ],
        description: "Data table",
    });

    // Ref: packages/react-ui/src/genui-lib/Table/schema.ts — ColSchema
    components.push(ComponentDef {
        name: "Col",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "type",
                type_annotation: "\"string\" | \"number\" | \"action\"",
                optional: true,
            },
        ],
        description: "Table column definition",
    });

    // =========================================================================
    // Charts (2D) — labels + series data
    // Ref: packages/react-ui/src/genui-lib/Charts/
    // =========================================================================

    // Ref: Charts/BarChartCondensed.ts
    components.push(ComponentDef {
        name: "BarChart",
        props: &[
            PropDef { name: "labels", type_annotation: "string[]", optional: false },
            PropDef { name: "series", type_annotation: "Series[]", optional: false },
            PropDef { name: "variant", type_annotation: "\"grouped\" | \"stacked\"", optional: true },
            PropDef { name: "xLabel", type_annotation: "string", optional: true },
            PropDef { name: "yLabel", type_annotation: "string", optional: true },
        ],
        description: "Vertical bars; use for comparing values across categories with one or more series",
    });

    // Ref: Charts/LineChartCondensed.ts
    components.push(ComponentDef {
        name: "LineChart",
        props: &[
            PropDef {
                name: "labels",
                type_annotation: "string[]",
                optional: false,
            },
            PropDef {
                name: "series",
                type_annotation: "Series[]",
                optional: false,
            },
            PropDef {
                name: "xLabel",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "yLabel",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Line chart for trends over time or ordered categories",
    });

    // Ref: Charts/AreaChartCondensed.ts
    components.push(ComponentDef {
        name: "AreaChart",
        props: &[
            PropDef {
                name: "labels",
                type_annotation: "string[]",
                optional: false,
            },
            PropDef {
                name: "series",
                type_annotation: "Series[]",
                optional: false,
            },
            PropDef {
                name: "variant",
                type_annotation: "\"default\" | \"stacked\"",
                optional: true,
            },
            PropDef {
                name: "xLabel",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "yLabel",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Filled area chart; stacked variant shows cumulative totals",
    });

    // Ref: Charts/RadarChart.ts
    components.push(ComponentDef {
        name: "RadarChart",
        props: &[
            PropDef {
                name: "labels",
                type_annotation: "string[]",
                optional: false,
            },
            PropDef {
                name: "series",
                type_annotation: "Series[]",
                optional: false,
            },
        ],
        description: "Spider/radar chart for multivariate comparison",
    });

    // Ref: Charts/HorizontalBarChart.ts
    components.push(ComponentDef {
        name: "HorizontalBarChart",
        props: &[
            PropDef {
                name: "labels",
                type_annotation: "string[]",
                optional: false,
            },
            PropDef {
                name: "series",
                type_annotation: "Series[]",
                optional: false,
            },
            PropDef {
                name: "variant",
                type_annotation: "\"grouped\" | \"stacked\"",
                optional: true,
            },
            PropDef {
                name: "xLabel",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "yLabel",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Horizontal bars; good for long category labels",
    });

    // Ref: Charts/Series.ts — data series sub-component
    components.push(ComponentDef {
        name: "Series",
        props: &[
            PropDef {
                name: "category",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "values",
                type_annotation: "number[]",
                optional: false,
            },
        ],
        description: "One data series",
    });

    // =========================================================================
    // Charts (1D) — slices data
    // =========================================================================

    // Ref: Charts/PieChart.ts
    components.push(ComponentDef {
        name: "PieChart",
        props: &[
            PropDef { name: "slices", type_annotation: "Slice[]", optional: false },
            PropDef { name: "variant", type_annotation: "\"pie\" | \"donut\"", optional: true },
        ],
        description: "Circular slices showing part-to-whole proportions; supports pie and donut variants",
    });

    // Ref: Charts/RadialChart.ts
    components.push(ComponentDef {
        name: "RadialChart",
        props: &[PropDef {
            name: "slices",
            type_annotation: "Slice[]",
            optional: false,
        }],
        description: "Radial bar chart for comparing a small number of values",
    });

    // Ref: Charts/SingleStackedBarChart.ts
    components.push(ComponentDef {
        name: "SingleStackedBarChart",
        props: &[PropDef {
            name: "slices",
            type_annotation: "Slice[]",
            optional: false,
        }],
        description: "Single horizontal stacked bar for quick part-to-whole comparison",
    });

    // Ref: Charts/Slice.ts — data slice sub-component
    components.push(ComponentDef {
        name: "Slice",
        props: &[
            PropDef {
                name: "category",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "value",
                type_annotation: "number",
                optional: false,
            },
        ],
        description: "One slice with label and numeric value",
    });

    // =========================================================================
    // Charts (Scatter)
    // =========================================================================

    // Ref: Charts/ScatterChart.ts
    components.push(ComponentDef {
        name: "ScatterChart",
        props: &[
            PropDef {
                name: "series",
                type_annotation: "ScatterSeries[]",
                optional: false,
            },
            PropDef {
                name: "xLabel",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "yLabel",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Scatter plot for showing correlation between two variables",
    });

    // Ref: Charts/ScatterSeries.ts
    components.push(ComponentDef {
        name: "ScatterSeries",
        props: &[
            PropDef {
                name: "name",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "points",
                type_annotation: "Point[]",
                optional: false,
            },
        ],
        description: "One scatter data series",
    });

    // Ref: Charts/Point.ts
    components.push(ComponentDef {
        name: "Point",
        props: &[
            PropDef {
                name: "x",
                type_annotation: "number",
                optional: false,
            },
            PropDef {
                name: "y",
                type_annotation: "number",
                optional: false,
            },
            PropDef {
                name: "z",
                type_annotation: "number",
                optional: true,
            },
        ],
        description: "One data point with x, y coordinates and optional z",
    });

    // =========================================================================
    // Forms
    // Ref: packages/react-ui/src/genui-lib/{Form,FormControl,Input,...}/
    // =========================================================================

    // Ref: packages/react-ui/src/genui-lib/Form/
    components.push(ComponentDef {
        name: "Form",
        props: &[
            PropDef {
                name: "name",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "fields",
                type_annotation: "FormControl[]",
                optional: false,
            },
            PropDef {
                name: "submit",
                type_annotation: "Button",
                optional: true,
            },
        ],
        description: "Form container with named fields and optional submit button",
    });

    // Ref: packages/react-ui/src/genui-lib/FormControl/
    components.push(ComponentDef {
        name: "FormControl",
        props: &[
            PropDef { name: "name", type_annotation: "string", optional: false },
            PropDef { name: "label", type_annotation: "Label", optional: true },
            PropDef { name: "input", type_annotation: "Input | TextArea | Select | DatePicker | Slider | CheckBoxGroup | RadioGroup | SwitchGroup", optional: false },
            PropDef { name: "rules", type_annotation: "{required?: boolean, email?: boolean, url?: boolean, numeric?: boolean, min?: number, max?: number, minLength?: number, maxLength?: number, pattern?: string}", optional: true },
        ],
        description: "Form field wrapper binding a name, label, input, and validation rules",
    });

    // Ref: packages/react-ui/src/genui-lib/Label/
    components.push(ComponentDef {
        name: "Label",
        props: &[
            PropDef {
                name: "text",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "description",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Field label with optional description",
    });

    // Ref: packages/react-ui/src/genui-lib/Input/
    components.push(ComponentDef {
        name: "Input",
        props: &[
            PropDef {
                name: "placeholder",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Single-line text input",
    });

    // Ref: packages/react-ui/src/genui-lib/TextArea/
    components.push(ComponentDef {
        name: "TextArea",
        props: &[
            PropDef {
                name: "placeholder",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "rows",
                type_annotation: "number",
                optional: true,
            },
        ],
        description: "Multi-line text area",
    });

    // Ref: packages/react-ui/src/genui-lib/Select/
    components.push(ComponentDef {
        name: "Select",
        props: &[
            PropDef {
                name: "items",
                type_annotation: "SelectItem[]",
                optional: false,
            },
            PropDef {
                name: "placeholder",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Dropdown select",
    });

    components.push(ComponentDef {
        name: "SelectItem",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "value",
                type_annotation: "string",
                optional: false,
            },
        ],
        description: "One select option",
    });

    // Ref: packages/react-ui/src/genui-lib/DatePicker/
    components.push(ComponentDef {
        name: "DatePicker",
        props: &[
            PropDef {
                name: "placeholder",
                type_annotation: "string",
                optional: true,
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Date picker input",
    });

    // Ref: packages/react-ui/src/genui-lib/Slider/
    components.push(ComponentDef {
        name: "Slider",
        props: &[
            PropDef {
                name: "min",
                type_annotation: "number",
                optional: true,
            },
            PropDef {
                name: "max",
                type_annotation: "number",
                optional: true,
            },
            PropDef {
                name: "step",
                type_annotation: "number",
                optional: true,
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "number",
                optional: true,
            },
        ],
        description: "Numeric slider",
    });

    // Ref: packages/react-ui/src/genui-lib/CheckBoxGroup/
    components.push(ComponentDef {
        name: "CheckBoxGroup",
        props: &[PropDef {
            name: "items",
            type_annotation: "CheckBoxItem[]",
            optional: false,
        }],
        description: "Group of checkboxes for multi-select",
    });

    components.push(ComponentDef {
        name: "CheckBoxItem",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "value",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "defaultChecked",
                type_annotation: "boolean",
                optional: true,
            },
        ],
        description: "One checkbox option",
    });

    // Ref: packages/react-ui/src/genui-lib/RadioGroup/
    components.push(ComponentDef {
        name: "RadioGroup",
        props: &[
            PropDef {
                name: "items",
                type_annotation: "RadioItem[]",
                optional: false,
            },
            PropDef {
                name: "defaultValue",
                type_annotation: "string",
                optional: true,
            },
        ],
        description: "Group of radio buttons for single-select",
    });

    components.push(ComponentDef {
        name: "RadioItem",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "value",
                type_annotation: "string",
                optional: false,
            },
        ],
        description: "One radio option",
    });

    // Ref: packages/react-ui/src/genui-lib/SwitchGroup/
    components.push(ComponentDef {
        name: "SwitchGroup",
        props: &[PropDef {
            name: "items",
            type_annotation: "SwitchItem[]",
            optional: false,
        }],
        description: "Group of toggle switches",
    });

    components.push(ComponentDef {
        name: "SwitchItem",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "value",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "defaultChecked",
                type_annotation: "boolean",
                optional: true,
            },
        ],
        description: "One toggle switch",
    });

    // =========================================================================
    // Buttons
    // Ref: packages/react-ui/src/genui-lib/{Button,Buttons}/
    // =========================================================================

    // Ref: packages/react-ui/src/genui-lib/Button/schema.ts
    components.push(ComponentDef {
        name: "Button",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "action",
                type_annotation: "ContinueConversation | OpenUrl | Custom",
                optional: true,
            },
            PropDef {
                name: "variant",
                type_annotation: "\"primary\" | \"secondary\" | \"tertiary\"",
                optional: true,
            },
            PropDef {
                name: "type",
                type_annotation: "\"normal\" | \"destructive\"",
                optional: true,
            },
            PropDef {
                name: "size",
                type_annotation: "\"extra-small\" | \"small\" | \"medium\" | \"large\"",
                optional: true,
            },
        ],
        description: "Clickable button with label and optional action",
    });

    // Ref: packages/react-ui/src/genui-lib/Buttons/
    components.push(ComponentDef {
        name: "Buttons",
        props: &[
            PropDef {
                name: "buttons",
                type_annotation: "Button[]",
                optional: false,
            },
            PropDef {
                name: "align",
                type_annotation: "\"start\" | \"center\" | \"end\"",
                optional: true,
            },
        ],
        description: "Row of buttons with alignment",
    });

    // =========================================================================
    // Data Display
    // Ref: packages/react-ui/src/genui-lib/{TagBlock,Tag}/
    // =========================================================================

    components.push(ComponentDef {
        name: "TagBlock",
        props: &[PropDef {
            name: "tags",
            type_annotation: "Tag[]",
            optional: false,
        }],
        description: "Group of tags",
    });

    components.push(ComponentDef {
        name: "Tag",
        props: &[
            PropDef {
                name: "label",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "variant",
                type_annotation: "\"default\" | \"success\" | \"warning\" | \"error\" | \"info\"",
                optional: true,
            },
        ],
        description: "Single tag/badge",
    });

    // =========================================================================
    // Chat-specific components (only in openuiChatLibrary)
    // Ref: packages/react-ui/src/genui-lib/{ListBlock,FollowUpBlock,SectionBlock}/
    // =========================================================================

    // Ref: packages/react-ui/src/genui-lib/ListBlock/
    components.push(ComponentDef {
        name: "ListBlock",
        props: &[
            PropDef {
                name: "items",
                type_annotation: "ListItem[]",
                optional: false,
            },
            PropDef {
                name: "variant",
                type_annotation: "\"ordered\" | \"unordered\"",
                optional: true,
            },
        ],
        description: "Ordered or unordered list",
    });

    components.push(ComponentDef {
        name: "ListItem",
        props: &[
            PropDef {
                name: "text",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "children",
                type_annotation: "any[]",
                optional: true,
            },
        ],
        description: "One list item with text and optional nested content",
    });

    // Ref: packages/react-ui/src/genui-lib/FollowUpBlock/
    components.push(ComponentDef {
        name: "FollowUpBlock",
        props: &[PropDef {
            name: "items",
            type_annotation: "FollowUpItem[]",
            optional: false,
        }],
        description: "Suggested follow-up questions",
    });

    components.push(ComponentDef {
        name: "FollowUpItem",
        props: &[
            PropDef {
                name: "text",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "action",
                type_annotation: "ContinueConversation | OpenUrl | Custom",
                optional: true,
            },
        ],
        description: "One follow-up suggestion with action",
    });

    // Ref: packages/react-ui/src/genui-lib/SectionBlock/
    components.push(ComponentDef {
        name: "SectionBlock",
        props: &[PropDef {
            name: "items",
            type_annotation: "SectionItem[]",
            optional: false,
        }],
        description: "Block of titled sections",
    });

    components.push(ComponentDef {
        name: "SectionItem",
        props: &[
            PropDef {
                name: "title",
                type_annotation: "string",
                optional: false,
            },
            PropDef {
                name: "children",
                type_annotation: "any[]",
                optional: false,
            },
        ],
        description: "One section with title and content",
    });

    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_components_have_names() {
        let components = all_components();
        for comp in &components {
            assert!(!comp.name.is_empty());
            // Component names should be PascalCase
            assert!(
                comp.name.chars().next().unwrap().is_uppercase(),
                "Component name '{}' should start with uppercase",
                comp.name
            );
        }
    }

    #[test]
    fn test_no_duplicate_component_names() {
        let components = all_components();
        let mut seen = std::collections::HashSet::new();
        for comp in &components {
            assert!(
                seen.insert(comp.name),
                "Duplicate component name: {}",
                comp.name
            );
        }
    }

    #[test]
    fn test_all_components_have_descriptions() {
        let components = all_components();
        for comp in &components {
            assert!(
                !comp.description.is_empty(),
                "Component '{}' has empty description",
                comp.name
            );
        }
    }

    #[test]
    fn test_prop_types_not_empty() {
        let components = all_components();
        for comp in &components {
            for prop in comp.props {
                assert!(
                    !prop.type_annotation.is_empty(),
                    "Component '{}' prop '{}' has empty type annotation",
                    comp.name,
                    prop.name
                );
            }
        }
    }

    #[test]
    fn test_stack_component_props() {
        let components = all_components();
        let stack = components.iter().find(|c| c.name == "Stack").unwrap();
        assert_eq!(stack.props[0].name, "children");
        assert!(!stack.props[0].optional);
        assert_eq!(stack.props[1].name, "direction");
        assert!(stack.props[1].optional);
    }

    #[test]
    fn test_table_component_props() {
        let components = all_components();
        let table = components.iter().find(|c| c.name == "Table").unwrap();
        assert_eq!(table.props[0].name, "columns");
        assert_eq!(table.props[1].name, "rows");
        assert!(!table.props[0].optional);
    }
}
