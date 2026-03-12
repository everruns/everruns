//! Component groups for organized prompt output.
//!
//! Groups organize components into logical categories in the generated prompt.
//! This mirrors the `componentGroups` array in the TypeScript library.
//!
//! Ref: packages/react-ui/src/genui-lib/openuiChatLibrary.tsx

/// A logical group of components for prompt organization.
///
/// Ref: packages/react-lang/src/library.ts `ComponentGroup` interface
pub struct ComponentGroup {
    /// Group display name (e.g., "Content", "Layout")
    pub name: &'static str,
    /// Component names belonging to this group
    pub components: Vec<&'static str>,
    /// Optional notes appended after the group's component signatures
    pub notes: Vec<&'static str>,
}

/// Returns the default component groups for the chat library.
///
/// Ref: packages/react-ui/src/genui-lib/openuiChatLibrary.tsx `openuiChatComponentGroups`
pub fn default_groups() -> Vec<ComponentGroup> {
    vec![
        // Ref: openuiChatLibrary.tsx — Content group
        ComponentGroup {
            name: "Content",
            components: vec![
                "CardHeader",
                "TextContent",
                "MarkDownRenderer",
                "Callout",
                "TextCallout",
                "Image",
                "ImageBlock",
                "ImageGallery",
                "CodeBlock",
                "Separator",
            ],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Tables group
        ComponentGroup {
            name: "Tables",
            components: vec!["Table", "Col"],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Charts (2D) group
        ComponentGroup {
            name: "Charts (2D)",
            components: vec![
                "BarChart",
                "LineChart",
                "AreaChart",
                "RadarChart",
                "HorizontalBarChart",
                "Series",
            ],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Charts (1D) group
        ComponentGroup {
            name: "Charts (1D)",
            components: vec!["PieChart", "RadialChart", "SingleStackedBarChart", "Slice"],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Charts (Scatter) group
        ComponentGroup {
            name: "Charts (Scatter)",
            components: vec!["ScatterChart", "ScatterSeries", "Point"],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Forms group
        ComponentGroup {
            name: "Forms",
            components: vec![
                "Form",
                "FormControl",
                "Label",
                "Input",
                "TextArea",
                "Select",
                "SelectItem",
                "DatePicker",
                "Slider",
                "CheckBoxGroup",
                "CheckBoxItem",
                "RadioGroup",
                "RadioItem",
                "SwitchGroup",
                "SwitchItem",
            ],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Buttons group
        ComponentGroup {
            name: "Buttons",
            components: vec!["Button", "Buttons"],
            notes: vec![
                "The `action` prop type accepts: ContinueConversation (sends message to LLM), OpenUrl (navigates to URL), or Custom (app-defined).",
            ],
        },
        // Ref: openuiChatLibrary.tsx — Lists & Follow-ups group
        ComponentGroup {
            name: "Lists & Follow-ups",
            components: vec!["ListBlock", "ListItem", "FollowUpBlock", "FollowUpItem"],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Sections group
        ComponentGroup {
            name: "Sections",
            components: vec!["SectionBlock", "SectionItem"],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Layout group
        ComponentGroup {
            name: "Layout",
            components: vec![
                "Stack",
                "Card",
                "Tabs",
                "TabItem",
                "Accordion",
                "AccordionItem",
                "Steps",
                "StepsItem",
                "Carousel",
            ],
            notes: vec![],
        },
        // Ref: openuiChatLibrary.tsx — Data Display group
        ComponentGroup {
            name: "Data Display",
            components: vec!["TagBlock", "Tag"],
            notes: vec![],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_groups_not_empty() {
        let groups = default_groups();
        assert!(!groups.is_empty());
    }

    #[test]
    fn test_no_empty_groups() {
        let groups = default_groups();
        for group in &groups {
            assert!(
                !group.components.is_empty(),
                "Group '{}' has no components",
                group.name
            );
        }
    }

    #[test]
    fn test_no_duplicate_components_across_groups() {
        let groups = default_groups();
        let mut seen = std::collections::HashSet::new();
        for group in &groups {
            for comp in &group.components {
                assert!(
                    seen.insert(*comp),
                    "Component '{}' appears in multiple groups",
                    comp
                );
            }
        }
    }
}
