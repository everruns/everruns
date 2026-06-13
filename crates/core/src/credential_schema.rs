// Shared credential form schema
//
// Declarative description of the credential fields an integration needs,
// rendered by the Settings UI and validated before saving. Shared by two
// front doors (see specs/providers.md "Credentials"):
//
// - Provider drivers (`DriverDescriptor::credential_schema`) — org-scoped
//   vendor accounts that power agent execution.
// - Connection providers (`ConnectionProvider::form_schema`) — user-scoped
//   accounts on external services used by tools.

use serde::Serialize;

/// Describes the form fields and instructions for entering a credential.
#[derive(Debug, Clone, Serialize)]
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
            fields: vec![FormField {
                name: "api_key".to_string(),
                label: "API Key".to_string(),
                field_type: FieldType::Password,
                required: true,
                placeholder: None,
                help_text: None,
            }],
            instructions_markdown: instructions_markdown.into(),
        }
    }
}

/// A single form field.
#[derive(Debug, Clone, Serialize)]
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
}

/// Input field type for rendering.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Masked password/secret input.
    Password,
    /// Plain text input.
    Text,
    /// URL input.
    Url,
}
