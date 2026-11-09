//! Presentation of MCP descriptive metadata, never protocol identifiers or schemas.

/// Make a narrow set of invisible format controls visible in descriptive text.
///
/// This is metadata hygiene, not a prompt-injection defense. Preserve the source
/// definition and invocation values; apply only to their descriptive projections.
/// Explicit matching avoids stripping joiners, variation selectors, emoji tags,
/// combining marks, or other characters needed by legitimate scripts. Work and
/// output growth are linear, with at most eight ASCII bytes per matched scalar.
pub fn visible_mcp_description(text: &str) -> String {
    let mut visible = String::with_capacity(text.len());
    for ch in text.chars() {
        let marker = match ch {
            '\u{061c}' => "[U+061C]", // Arabic letter mark
            '\u{200b}' => "[U+200B]", // zero-width space
            '\u{200e}' => "[U+200E]", // left-to-right mark
            '\u{200f}' => "[U+200F]", // right-to-left mark
            '\u{202a}' => "[U+202A]", // left-to-right embedding
            '\u{202b}' => "[U+202B]", // right-to-left embedding
            '\u{202c}' => "[U+202C]", // pop directional formatting
            '\u{202d}' => "[U+202D]", // left-to-right override
            '\u{202e}' => "[U+202E]", // right-to-left override
            '\u{2060}' => "[U+2060]", // word joiner
            '\u{2066}' => "[U+2066]", // left-to-right isolate
            '\u{2067}' => "[U+2067]", // right-to-left isolate
            '\u{2068}' => "[U+2068]", // first strong isolate
            '\u{2069}' => "[U+2069]", // pop directional isolate
            '\u{feff}' => "[U+FEFF]", // byte order mark / zero-width no-break space
            _ => {
                visible.push(ch);
                continue;
            }
        };
        visible.push_str(marker);
    }
    visible
}

/// Project only title/description annotations at recognized JSON Schema locations.
///
/// Names, references, validation keywords and literal instance data remain exact.
/// Unknown extension payloads are opaque. This best-effort presentation pass visits
/// at most 16,384 schema nodes and 64 nested levels; deeper/excess metadata is left
/// unchanged, not treated as safe. These limits do not reject otherwise valid tools.
pub fn project_schema_descriptions(schema: &serde_json::Value) -> serde_json::Value {
    fn visit(schema: &mut serde_json::Value, depth: usize, remaining: &mut usize) {
        if *remaining == 0 {
            return;
        }
        *remaining -= 1;
        if depth == 64 {
            return;
        }
        let Some(object) = schema.as_object_mut() else {
            return;
        };
        for key in ["title", "description"] {
            if let Some(serde_json::Value::String(text)) = object.get_mut(key) {
                *text = visible_mcp_description(text);
            }
        }
        // Map keys are property names, regexes or definition identifiers, not metadata.
        for key in [
            "properties",
            "patternProperties",
            "$defs",
            "definitions",
            "dependentSchemas",
            "dependencies",
        ] {
            if let Some(children) = object
                .get_mut(key)
                .and_then(serde_json::Value::as_object_mut)
            {
                for child in children.values_mut() {
                    if *remaining == 0 {
                        return;
                    }
                    // Legacy dependencies can be arrays of property names.
                    if child.is_object() || child.is_boolean() {
                        visit(child, depth + 1, remaining);
                    }
                }
            }
        }
        for key in [
            "items",
            "additionalItems",
            "additionalProperties",
            "contains",
            "propertyNames",
            "not",
            "if",
            "then",
            "else",
            "unevaluatedItems",
            "unevaluatedProperties",
            "contentSchema",
        ] {
            if let Some(child) = object.get_mut(key) {
                if key == "items" && child.is_array() {
                    if let Some(children) = child.as_array_mut() {
                        for child in children {
                            if *remaining == 0 {
                                return;
                            }
                            visit(child, depth + 1, remaining);
                        }
                    }
                } else {
                    visit(child, depth + 1, remaining);
                }
            }
        }
        for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
            if let Some(children) = object
                .get_mut(key)
                .and_then(serde_json::Value::as_array_mut)
            {
                for child in children {
                    if *remaining == 0 {
                        return;
                    }
                    visit(child, depth + 1, remaining);
                }
            }
        }
    }
    let mut projected = schema.clone();
    visit(&mut projected, 0, &mut 16_384);
    projected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_projection_visits_annotations_not_arbitrary_instance_objects() {
        use serde_json::json;
        let annotation = json!({"title": "T\u{202e}", "description": "D\u{200b}"});
        let literal = json!({"title": "literal\u{202e}", "description": "literal\u{200b}", "properties": {"description": annotation}});
        let schema = json!({
            "title": "T\u{202e}", "description": "D\u{200b}",
            "properties": {"description": annotation, "name\u{200b}": {
                "default": literal, "const": literal, "enum": [literal], "examples": [literal],
                "x-extension": annotation, "$ref": "#/$defs/name\u{200b}",
                "required": ["description\u{200b}"], "pattern": "x\u{200b}"
            }},
            "dependencies": {"description": ["name\u{200b}"]},
            "x-extension": {"properties": {"field": annotation}}, "default": literal
        });
        let projected = project_schema_descriptions(&schema);
        assert_eq!(projected["title"], "T[U+202E]");
        assert_eq!(
            projected["properties"]["description"]["description"],
            "D[U+200B]"
        );
        for key in ["default", "dependencies", "x-extension"] {
            assert_eq!(projected[key], schema[key]);
        }
        assert_eq!(
            projected["properties"]["name\u{200b}"],
            schema["properties"]["name\u{200b}"]
        );
        assert_eq!(schema["description"], "D\u{200b}");
        assert_eq!(project_schema_descriptions(&projected), projected);
    }

    #[test]
    fn recognized_subschema_locations_are_projected() {
        use serde_json::json;
        let child = json!({"description":"D\u{202e}"});
        for key in [
            "properties",
            "patternProperties",
            "$defs",
            "definitions",
            "dependentSchemas",
            "dependencies",
        ] {
            let schema = json!({key: {"name": child}});
            assert_eq!(
                project_schema_descriptions(&schema)[key]["name"]["description"],
                "D[U+202E]",
                "{key}"
            );
        }
        for key in [
            "items",
            "additionalItems",
            "additionalProperties",
            "contains",
            "propertyNames",
            "not",
            "if",
            "then",
            "else",
            "unevaluatedItems",
            "unevaluatedProperties",
            "contentSchema",
        ] {
            let schema = json!({key: child});
            assert_eq!(
                project_schema_descriptions(&schema)[key]["description"],
                "D[U+202E]",
                "{key}"
            );
        }
        for key in ["items", "prefixItems", "allOf", "anyOf", "oneOf"] {
            let schema = json!({key: [child, true, false]});
            let projected = project_schema_descriptions(&schema);
            assert_eq!(projected[key][0]["description"], "D[U+202E]", "{key}");
            assert_eq!(projected[key][1], true);
            assert_eq!(projected[key][2], false);
        }
    }

    #[test]
    fn schema_projection_stops_at_explicit_depth_and_node_limits() {
        use serde_json::json;
        let leaf = json!({"description":"D\u{202e}"});
        let mut schema = leaf.clone();
        for _ in 0..65 {
            schema = json!({"items": schema});
        }
        let projected = project_schema_descriptions(&schema);
        assert_eq!(projected, schema);
        let wide = json!({"allOf": vec![leaf; 16_385]});
        let projected = project_schema_descriptions(&wide);
        assert_eq!(projected["allOf"][16_382]["description"], "D[U+202E]");
        assert_eq!(projected["allOf"][16_383], wide["allOf"][16_383]);
    }

    #[test]
    fn marks_exact_controls_and_is_idempotent() {
        for codepoint in [
            0x061c, 0x200b, 0x200e, 0x200f, 0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2060, 0x2066,
            0x2067, 0x2068, 0x2069, 0xfeff,
        ] {
            let input = format!("before{}after", char::from_u32(codepoint).unwrap());
            let expected = format!("before[U+{codepoint:04X}]after");
            assert_eq!(visible_mcp_description(&input), expected);
            assert_eq!(visible_mcp_description(&expected), expected);
        }
    }

    #[test]
    fn preserves_scripts_joiners_emoji_whitespace_and_adjacent_codepoints() {
        let text = "Українська العربية עברית 中文 हिन्दी e\u{301} می\u{200c}روم क्\u{200d}ष 👩\u{200d}💻 ❤️ 🏴\u{e0067}\u{e0062}\u{e007f}\n\t\r \u{00a0}\u{200a}\u{2061}\u{2065}\u{206a}\u{fe00}\u{ffff}";
        assert_eq!(visible_mcp_description(text), text);
        assert_eq!(visible_mcp_description(""), "");
    }

    #[test]
    fn repeated_controls_have_bounded_expansion() {
        let text = "\u{202e}".repeat(10_000);
        assert_eq!(visible_mcp_description(&text), "[U+202E]".repeat(10_000));
    }
}
