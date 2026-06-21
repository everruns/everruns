// Shared credential form schema
//
// Declarative description of the credential fields an integration needs,
// rendered by the Settings UI and validated before saving. Shared by two
// front doors (see specs/providers.md "Credentials"):
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
            let complete = group_fields
                .iter()
                .all(|f| !f.required || filled(&f.name));
            if touched && !complete {
                for field in group_fields.iter().filter(|f| f.required && !filled(&f.name)) {
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

    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(document) {
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
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn assemble_lone_api_key_stays_raw() {
        let doc = assemble_credential_document(&map(&[("api_key", "sk-123")]));
        assert_eq!(doc.as_deref(), Some("sk-123"));
    }

    #[test]
    fn assemble_multi_field_is_json_and_drops_empty() {
        let doc = assemble_credential_document(&map(&[
            ("access_key_id", "AKID"),
            ("secret_access_key", "SECRET"),
            ("region", ""),
        ]))
        .expect("doc");
        let parsed: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(parsed["access_key_id"], "AKID");
        assert_eq!(parsed["secret_access_key"], "SECRET");
        assert!(parsed.get("region").is_none(), "empty fields dropped");
    }

    #[test]
    fn assemble_nothing_is_none() {
        assert!(assemble_credential_document(&map(&[("api_key", "  ")])).is_none());
        assert!(assemble_credential_document(&BTreeMap::new()).is_none());
    }

    #[test]
    fn parse_round_trips_multi_field() {
        let doc = assemble_credential_document(&map(&[
            ("access_key_id", "AKID"),
            ("secret_access_key", "SECRET"),
        ]))
        .unwrap();
        let fields = parse_credential_document(Some(&doc));
        assert_eq!(fields["access_key_id"], "AKID");
        assert_eq!(fields["secret_access_key"], "SECRET");
    }

    #[test]
    fn parse_raw_key_becomes_api_key_field() {
        let fields = parse_credential_document(Some("sk-raw-key"));
        assert_eq!(fields["api_key"], "sk-raw-key");
    }

    #[test]
    fn parse_legacy_json_oauth_document() {
        // Existing MAI rows stored OAuth as a JSON document; they must still
        // parse into discrete fields after the migration to typed credentials.
        let legacy = r#"{"tenant_id":"t","client_id":"c","client_secret":"s"}"#;
        let fields = parse_credential_document(Some(legacy));
        assert_eq!(fields["tenant_id"], "t");
        assert_eq!(fields["client_secret"], "s");
    }

    #[test]
    fn parse_none_is_empty() {
        assert!(parse_credential_document(None).is_empty());
    }

    #[test]
    fn validate_required_field_missing() {
        let schema = CredentialFormSchema::api_key("");
        let errors = schema.validate(&BTreeMap::new());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("API Key"));
    }

    #[test]
    fn validate_grouped_either_or() {
        // MAI-style schema: an API-key group OR an OAuth group.
        let schema = CredentialFormSchema {
            fields: vec![
                FormField::password("api_key", "API Key")
                    .required()
                    .in_group("API key"),
                FormField::text("tenant_id", "Tenant ID")
                    .required()
                    .in_group("OAuth"),
                FormField::text("client_id", "Client ID")
                    .required()
                    .in_group("OAuth"),
                FormField::password("client_secret", "Client Secret")
                    .required()
                    .in_group("OAuth"),
            ],
            instructions_markdown: String::new(),
        };

        // Neither group filled → error.
        assert!(!schema.validate(&BTreeMap::new()).is_empty());
        // API key alone → valid.
        assert!(schema.validate(&map(&[("api_key", "k")])).is_empty());
        // Full OAuth group → valid.
        assert!(
            schema
                .validate(&map(&[
                    ("tenant_id", "t"),
                    ("client_id", "c"),
                    ("client_secret", "s"),
                ]))
                .is_empty()
        );
        // Partial OAuth group → error naming the missing fields.
        let errors = schema.validate(&map(&[("tenant_id", "t")]));
        assert!(errors.iter().any(|e| e.contains("Client ID")));
        assert!(errors.iter().any(|e| e.contains("Client Secret")));
    }
}
