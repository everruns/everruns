// Shared credential form schema
//
// Declarative description of the credential fields an integration needs,
// rendered by the Settings UI and validated before saving. Shared by two
// front doors (see knowledge/foundations/providers.md "Credentials"):
//
// - Provider drivers (`DriverDescriptor::credential_schema`) — org-scoped
//   vendor accounts that power agent execution.
// - Connectors (`Connector::form_schema`) — user-scoped
//   accounts on external services used by tools.
//
// Multi-field credentials (Bedrock AWS keys, Microsoft Entra ID OAuth) are
// declared as discrete typed fields rather than a single opaque password the
// operator hand-authors as JSON. The submitted field map is assembled into a
// single credential document (`assemble_credential_document`) that is stored in
// the existing envelope-encrypted credential field, and parsed back into a
// typed field map at driver-construction time (`parse_credential_document`).
// Keeping the storage shape a JSON document means existing Bedrock/MAI rows —
// which already store such a document — keep resolving unchanged.

use std::collections::BTreeMap;

use serde::Serialize;

/// Describes the form fields and instructions for entering a credential.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CredentialFormSchema {
    /// Input fields to render.
    pub fields: Vec<FormField>,
    /// Markdown instructions shown above the form (how to get the key, etc.).
    pub instructions_markdown: String,
}

impl CredentialFormSchema {
    /// Schema with no fields (keyless integrations, e.g. test simulators).
    pub fn empty() -> Self {
        Self {
            fields: Vec::new(),
            instructions_markdown: String::new(),
        }
    }

    /// The common single-field API key schema.
    pub fn api_key(instructions_markdown: impl Into<String>) -> Self {
        Self {
            fields: vec![FormField::password("api_key", "API Key").required()],
            instructions_markdown: instructions_markdown.into(),
        }
    }

    /// Whether the schema declares any mutually-exclusive credential groups
    /// (e.g. "API key" *or* "OAuth"). When it does, at least one complete group
    /// is required by [`CredentialFormSchema::validate`].
    pub fn has_groups(&self) -> bool {
        self.fields.iter().any(|f| f.group.is_some())
    }

    /// Validate a submitted field map against this schema.
    ///
    /// Rules:
    /// - Every ungrouped required field must be present and non-empty.
    /// - Fields sharing a `group` label form a mutually-exclusive alternative
    ///   (e.g. MAI's "API key" group vs its "Entra ID OAuth" group). A group is
    ///   *touched* when any of its fields has a value; a touched group must have
    ///   all of its required fields filled (no partially-entered OAuth blocks).
    /// - When the schema declares any groups, at least one group must be
    ///   complete, so a provider cannot be saved with no credential at all.
    ///
    /// Returns the list of human-readable validation errors; empty means valid.
    /// Unknown keys are ignored — drivers read only the fields they declare.
    pub fn validate(&self, fields: &BTreeMap<String, String>) -> Vec<String> {
        let filled = |name: &str| fields.get(name).is_some_and(|v| !v.trim().is_empty());
        let mut errors = Vec::new();

        // Ungrouped required fields are always mandatory.
        for field in self.fields.iter().filter(|f| f.group.is_none()) {
            if field.required && !filled(&field.name) {
                errors.push(format!("{} is required.", field.label));
            }
        }

        if !self.has_groups() {
            return errors;
        }

        // Grouped fields: collect group labels in declaration order.
        let mut group_labels: Vec<&str> = Vec::new();
        for field in &self.fields {
            if let Some(group) = field.group.as_deref()
                && !group_labels.contains(&group)
            {
                group_labels.push(group);
            }
        }

        let mut any_group_complete = false;
        for label in group_labels {
            let group_fields: Vec<&FormField> = self
                .fields
                .iter()
                .filter(|f| f.group.as_deref() == Some(label))
                .collect();
            let touched = group_fields.iter().any(|f| filled(&f.name));
            let complete = group_fields.iter().all(|f| !f.required || filled(&f.name));
            if touched && !complete {
                for field in group_fields
                    .iter()
                    .filter(|f| f.required && !filled(&f.name))
                {
                    errors.push(format!("{} ({}) is required.", field.label, label));
                }
            }
            if complete && touched {
                any_group_complete = true;
            }
        }

        if !any_group_complete && errors.is_empty() {
            errors.push("Provide credentials for one of the available methods.".to_string());
        }

        errors
    }
}

/// A single form field.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FormField {
    /// Field name used as the key when submitting (e.g. "api_key").
    pub name: String,
    /// Label shown next to the input.
    pub label: String,
    /// Input type.
    pub field_type: FieldType,
    /// Whether the field is required.
    pub required: bool,
    /// Placeholder text inside the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Help text shown below the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
    /// Default value the UI pre-fills (e.g. an OAuth scope or AWS region). The
    /// stored credential omits unfilled optional fields, so drivers still apply
    /// their own defaults; this only seeds the form input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// Mutually-exclusive group label this field belongs to. Fields sharing a
    /// label are one alternative credential method (e.g. "API key" vs "OAuth");
    /// ungrouped fields are always part of the credential. `None` for the
    /// common single-method case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl FormField {
    /// A field of the given type with no extra metadata.
    fn new(name: impl Into<String>, label: impl Into<String>, field_type: FieldType) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            field_type,
            required: false,
            placeholder: None,
            help_text: None,
            default_value: None,
            group: None,
        }
    }

    /// A masked password/secret field.
    pub fn password(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, FieldType::Password)
    }

    /// A plain text field.
    pub fn text(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, FieldType::Text)
    }

    /// Mark the field as required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set placeholder text.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set help text shown below the input.
    pub fn with_help(mut self, help_text: impl Into<String>) -> Self {
        self.help_text = Some(help_text.into());
        self
    }

    /// Set a default value the form pre-fills.
    pub fn with_default(mut self, default_value: impl Into<String>) -> Self {
        self.default_value = Some(default_value.into());
        self
    }

    /// Assign the field to a mutually-exclusive credential group.
    pub fn in_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }
}

/// Input field type for rendering.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Masked password/secret input.
    #[default]
    Password,
    /// Plain text input.
    Text,
    /// URL input.
    Url,
}

/// Assemble a submitted credential field map into the single document string
/// that is envelope-encrypted and stored.
///
/// Empty values are dropped so unfilled optional fields are not persisted. A
/// lone `api_key` field is stored as the raw key (the long-standing simple
/// shape), so single-key providers and the dev env-var fallback keep their
/// exact storage format. Any other field set is stored as a deterministic JSON
/// object keyed by field name — the shape Bedrock and MAI already use.
///
/// Returns `None` when nothing was supplied (no credential to store).
pub fn assemble_credential_document(fields: &BTreeMap<String, String>) -> Option<String> {
    let non_empty: BTreeMap<&str, &str> = fields
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    match non_empty.len() {
        0 => None,
        1 if non_empty.contains_key("api_key") => Some(non_empty["api_key"].to_string()),
        _ => Some(serde_json::to_string(&non_empty).expect("string map serializes")),
    }
}

/// Parse a stored credential document back into a typed field map.
///
/// A JSON object of string values (Bedrock/MAI multi-field documents) parses
/// into its fields. Anything else — a raw API key, or a legacy non-JSON
/// credential — is treated as the `api_key` field, so existing single-key
/// providers keep resolving without re-encryption.
pub fn parse_credential_document(document: Option<&str>) -> BTreeMap<String, String> {
    let Some(document) = document else {
        return BTreeMap::new();
    };

    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(document)
    {
        let mut fields = BTreeMap::new();
        for (key, value) in map {
            if let serde_json::Value::String(s) = value {
                fields.insert(key, s);
            }
        }
        // A JSON object that carried no string fields is not a credential
        // document we understand; fall back to treating it as an opaque key.
        if !fields.is_empty() {
            return fields;
        }
    }

    BTreeMap::from([("api_key".to_string(), document.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn assembly_uses_literal_legacy_and_deterministic_json_formats() {
        for (fields, expected) in [
            (vec![], None),
            (vec![("api_key", " \t\n"), ("region", "")], None),
            (
                vec![("api_key", " key with spaces "), ("region", " \n")],
                Some(" key with spaces "),
            ),
            (
                vec![
                    ("secret_access_key", "SECRET"),
                    ("region", ""),
                    ("access_key_id", "AKID"),
                ],
                Some(r#"{"access_key_id":"AKID","secret_access_key":"SECRET"}"#),
            ),
            (vec![("tenant_id", "one")], Some(r#"{"tenant_id":"one"}"#)),
            (
                vec![("api_key", "quote\"slash\\line\n"), ("region", "west")],
                Some(r#"{"api_key":"quote\"slash\\line\n","region":"west"}"#),
            ),
        ] {
            assert_eq!(
                assemble_credential_document(&map(&fields)).as_deref(),
                expected,
                "{fields:?}"
            );
        }
    }

    #[test]
    fn parsing_preserves_legacy_documents_and_opaque_fallback_exactly() {
        assert_eq!(parse_credential_document(None), BTreeMap::new());
        assert_eq!(
            parse_credential_document(Some(
                r#"{"tenant_id":"t","client_id":"c","client_secret":"s"}"#
            )),
            map(&[
                ("tenant_id", "t"),
                ("client_id", "c"),
                ("client_secret", "s")
            ])
        );
        assert_eq!(
            parse_credential_document(Some(
                r#"{"access_key_id":"AKID","secret_access_key":"SECRET","ignored":42,"empty":""}"#
            )),
            map(&[
                ("access_key_id", "AKID"),
                ("secret_access_key", "SECRET"),
                ("empty", "")
            ])
        );
        for opaque in [
            "sk-raw-key",
            "",
            "  ",
            "not { json",
            "{}",
            r#"{"number":42}"#,
            "[1,2]",
            "null",
            r#""json string""#,
        ] {
            assert_eq!(
                parse_credential_document(Some(opaque)),
                map(&[("api_key", opaque)]),
                "opaque {opaque:?}"
            );
        }
    }

    #[test]
    fn single_method_schema_requires_nonblank_key_but_accepts_unknown_optional_fields() {
        let schema = CredentialFormSchema::api_key("Get a key");
        assert_eq!(
            serde_json::to_value(&schema).unwrap(),
            serde_json::json!({
                "fields": [{"name":"api_key","label":"API Key","field_type":"password","required":true}],
                "instructions_markdown":"Get a key"
            })
        );
        for fields in [
            map(&[]),
            map(&[("api_key", " \t\n")]),
            map(&[("unknown", "value")]),
        ] {
            assert_eq!(schema.validate(&fields), ["API Key is required."]);
        }
        assert!(
            schema
                .validate(&map(&[("api_key", " key "), ("unknown", "value")]))
                .is_empty()
        );
        assert!(
            CredentialFormSchema::empty()
                .validate(&map(&[("unknown", "value")]))
                .is_empty()
        );
    }

    #[test]
    fn grouped_validation_requires_complete_touched_methods_in_declaration_order() {
        let schema = CredentialFormSchema {
            fields: vec![
                FormField::text("endpoint", "Endpoint").required(),
                FormField::password("api_key", "API Key")
                    .required()
                    .in_group("API key"),
                FormField::text("tenant_id", "Tenant ID")
                    .required()
                    .in_group("OAuth"),
                FormField::text("scope", "Scope").in_group("OAuth"),
                FormField::text("client_id", "Client ID")
                    .required()
                    .in_group("OAuth"),
                FormField::password("client_secret", "Client Secret")
                    .required()
                    .in_group("OAuth"),
            ],
            instructions_markdown: String::new(),
        };
        for (fields, expected) in [
            (vec![], vec!["Endpoint is required."]),
            (
                vec![("endpoint", "url")],
                vec!["Provide credentials for one of the available methods."],
            ),
            (vec![("endpoint", "url"), ("api_key", "k")], vec![]),
            (vec![("api_key", "k")], vec!["Endpoint is required."]),
            (
                vec![
                    ("endpoint", "url"),
                    ("tenant_id", "t"),
                    ("client_id", "c"),
                    ("client_secret", "s"),
                ],
                vec![],
            ),
            (
                vec![("endpoint", "url"), ("tenant_id", "t")],
                vec![
                    "Client ID (OAuth) is required.",
                    "Client Secret (OAuth) is required.",
                ],
            ),
            (
                vec![
                    ("endpoint", "url"),
                    ("api_key", "k"),
                    ("tenant_id", "t"),
                    ("client_id", " \n"),
                ],
                vec![
                    "Client ID (OAuth) is required.",
                    "Client Secret (OAuth) is required.",
                ],
            ),
            (
                vec![("endpoint", "url"), ("scope", "optional-but-touched")],
                vec![
                    "Tenant ID (OAuth) is required.",
                    "Client ID (OAuth) is required.",
                    "Client Secret (OAuth) is required.",
                ],
            ),
        ] {
            assert_eq!(schema.validate(&map(&fields)), expected, "{fields:?}");
        }
    }
}
