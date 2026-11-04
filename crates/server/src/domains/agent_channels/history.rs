//! Restore the same public configuration Chat can edit, preserving current authority.
use crate::domains::common::{CommandError, Ctx};
use serde_json::{Map, Value};

fn fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "public_chat" => &["branding", "rate_limit_per_minute"],
        "schedule" => &["cron_expression", "timezone", "message", "session_mode"],
        "webhook" => &["message", "session_mode"],
        "ag_ui" => &["generic_tool_text", "rate_limit_per_minute"],
        "fcp" => &[
            "handshake",
            "rate_limit_per_minute",
            "response_timeout_seconds",
        ],
        "slack" => &["session_strategy", "response_policy"],
        "api" => &[
            "tool_activity_text",
            "rate_limit_per_minute",
            "session_binding",
        ],
        "work_chat" => &["interface_mode", "execution"],
        "voice" => &[
            "model",
            "voice",
            "language",
            "greeting",
            "filler",
            "filler_after_ms",
            "speaking_style",
            "turn_detection",
            "interruption",
        ],
        _ => &[],
    }
}
pub(crate) fn public_config(kind: &str, config: &Value) -> Value {
    Value::Object(
        fields(kind)
            .iter()
            .map(|key| {
                let value = config.get(*key).cloned().unwrap_or(Value::Null);
                let value = if kind == "work_chat" && *key == "execution" {
                    public_config("api", &value)
                } else {
                    value
                };
                ((*key).to_string(), value)
            })
            .collect(),
    )
}
fn apply(kind: &str, current: &mut Value, public: &Value) {
    if !current.is_object() {
        *current = Value::Object(Map::new());
    }
    for key in fields(kind) {
        if kind == "work_chat" && *key == "execution" {
            let value = current
                .as_object_mut()
                .unwrap()
                .entry("execution")
                .or_insert(Value::Object(Map::new()));
            apply(
                "api",
                value,
                public.get("execution").unwrap_or(&Value::Null),
            );
        } else if let Some(value) = public.get(*key) {
            if value.is_null() {
                current.as_object_mut().unwrap().remove(*key);
            } else {
                current
                    .as_object_mut()
                    .unwrap()
                    .insert((*key).into(), value.clone());
            }
        }
    }
}
pub(crate) async fn prepare_restore(ctx: &Ctx, params: &Value) -> Result<Value, CommandError> {
    let mut params = params.clone();
    if let Some(public) = params.get("channel_config").cloned() {
        let agent_id = params
            .get("agent_id")
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::bad_request("Missing channel agent"))?;
        let channel_id = params
            .get("channel_id")
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::bad_request("Missing channel id"))?;
        let agent = super::commands::resolve_agent(ctx, agent_id).await?;
        let row = ctx
            .db
            .get_agent_channel(ctx.org_id(), agent.id.uuid(), channel_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        let kind = row.channel_type.clone();
        let mut current = super::commands::decrypted_config(ctx, row)?;
        apply(&kind, &mut current, &public);
        params["channel_config"] = current;
    }
    Ok(params)
}
