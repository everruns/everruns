//! Provider-neutral structured output: ask the model for a reply that
//! validates against a JSON Schema, enforced by the provider rather than by
//! prompting for JSON and hoping.
//!
//! Set [`LlmCallConfig::response_format`](crate::driver_registry::LlmCallConfig::response_format).
//! Each driver maps it to its native control (OpenAI Responses `text.format`,
//! Chat Completions `response_format`, Anthropic `output_config.format`,
//! Gemini `responseJsonSchema`). A driver without one reports it through
//! [`ChatDriver::supports_response_format`](crate::driver_registry::ChatDriver::supports_response_format),
//! and the provider fails the call with a configuration error instead of
//! silently dropping the constraint.
//!
//! Scope decision (EVE-1116): this is for single completions (evals, utility
//! calls, Framework `Model` users). Agent turns never set it: an agent's final
//! message stays prose.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Constrains the model's reply format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResponseFormat {
    /// The reply is a JSON document matching `schema`.
    JsonSchema(JsonSchemaFormat),
}

/// A named JSON Schema the reply must satisfy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonSchemaFormat {
    /// Schema name (OpenAI requires one: `^[a-zA-Z0-9_-]{1,64}$`).
    pub name: String,
    /// The JSON Schema itself.
    pub schema: Value,
    /// Ask for strict schema adherence where the provider distinguishes it
    /// (OpenAI). Strict schemas must set `additionalProperties: false` and list
    /// every property as required.
    pub strict: bool,
}

impl ResponseFormat {
    /// A strict JSON Schema format.
    pub fn json_schema(name: impl Into<String>, schema: Value) -> Self {
        Self::JsonSchema(JsonSchemaFormat {
            name: name.into(),
            schema,
            strict: true,
        })
    }

    /// The same format with strict adherence switched off.
    pub fn non_strict(self) -> Self {
        match self {
            Self::JsonSchema(format) => Self::JsonSchema(JsonSchemaFormat {
                strict: false,
                ..format
            }),
        }
    }

    /// The schema, whatever the variant.
    pub fn schema(&self) -> &Value {
        match self {
            Self::JsonSchema(format) => &format.schema,
        }
    }

    /// OpenAI Responses `text.format` object.
    #[cfg(feature = "http")]
    pub(crate) fn responses_text_format(&self) -> Value {
        match self {
            Self::JsonSchema(format) => serde_json::json!({
                "type": "json_schema",
                "name": format.name,
                "schema": format.schema,
                "strict": format.strict,
            }),
        }
    }

    /// OpenAI Chat Completions `response_format` object.
    #[cfg(feature = "http")]
    pub(crate) fn chat_completions_response_format(&self) -> Value {
        match self {
            Self::JsonSchema(format) => serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": format.name,
                    "schema": format.schema,
                    "strict": format.strict,
                },
            }),
        }
    }
}
