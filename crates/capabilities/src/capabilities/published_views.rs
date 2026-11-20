use super::{Capability, CapabilityStatus};
use async_trait::async_trait;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};

pub const PUBLISHED_VIEWS_ID: &str = "published_views";
pub const PUBLISH_VIEW_TOOL: &str = "publish_view";

pub struct PublishedViews;

impl Capability for PublishedViews {
    fn id(&self) -> &str {
        PUBLISHED_VIEWS_ID
    }

    fn name(&self) -> &str {
        "Published views"
    }

    fn description(&self) -> &str {
        "Update the configured Agent view with the results of a successful run."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("panel-right")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Use publish_view near the end of a successful run when asked to update the configured view. Use only the data keys specified by the author. Supply a complete data snapshot for the intended audience. Never include secrets or private session information. An Agent Trigger updates the Agent-wide view; other turns update only their own conversation. Supply layout only when the author asks for a generated layout and has granted layout publishing.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(PublishView)]
    }
}

struct PublishView;

#[async_trait]
impl Tool for PublishView {
    fn name(&self) -> &str {
        PUBLISH_VIEW_TOOL
    }

    fn description(&self) -> &str {
        "Stage a view update. The view is updated when the turn succeeds. Data is a complete replacement using the configured field names."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object", "required": ["data"], "additionalProperties": false,
            "properties": {
                "data": { "type": "object", "description": "Fields the author authorized for the view." },
                "layout": {
                    "type": "object", "description": "Optional replacement layout. Requires author permission.",
                    "oneOf": [
                        {
                            "type": "object", "required": ["format", "operations"], "additionalProperties": false,
                            "properties": { "format": { "const": "a2ui" }, "operations": { "type": "array", "items": { "type": "object" } } }
                        },
                        {
                            "type": "object", "required": ["format", "source"], "additionalProperties": false,
                            "properties": { "format": { "const": "html" }, "source": { "type": "string" } }
                        }
                    ]
                }
            }
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        if arguments.get("data").is_none_or(|data| !data.is_object())
            || serde_json::to_vec(&arguments).is_ok_and(|bytes| bytes.len() > 80 * 1024)
        {
            return ToolExecutionResult::tool_error(
                "Provide a data object and keep the entire update under 80 KiB.".to_owned(),
            );
        }
        // A durable listener commits the last staged result of a successful turn.
        ToolExecutionResult::success(json!({
            "status": "staged_until_turn_completes",
            "view_update": arguments,
        }))
    }
}
