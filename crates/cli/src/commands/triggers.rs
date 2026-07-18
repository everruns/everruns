use crate::commands::api::ApiClient;
use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum SessionMode {
    SharedSession,
    SessionPerInvocation,
}

impl SessionMode {
    fn as_api_value(self) -> &'static str {
        match self {
            Self::SharedSession => "shared_session",
            Self::SessionPerInvocation => "session_per_invocation",
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum TriggersCommand {
    /// List schedule triggers
    List,
    /// Create a schedule trigger
    Create {
        /// Cron expression (5-field or 7-field)
        #[arg(long = "cron", alias = "cron-expression")]
        cron_expression: String,
        /// IANA timezone for cron evaluation
        #[arg(long, default_value = "UTC")]
        timezone: String,
        /// Whether runs reuse a session or create a new session
        #[arg(long, value_enum, default_value = "shared-session")]
        session_mode: SessionMode,
        /// Message sent when the trigger fires
        #[arg(long)]
        message: String,
        /// Create the trigger disabled
        #[arg(long)]
        disabled: bool,
    },
    /// Update a schedule trigger
    Update {
        /// Trigger ID (e.g. trg_xxx)
        trigger: String,
        #[arg(long = "cron", alias = "cron-expression")]
        cron_expression: Option<String>,
        #[arg(long)]
        timezone: Option<String>,
        #[arg(long, value_enum)]
        session_mode: Option<SessionMode>,
        #[arg(long)]
        message: Option<String>,
    },
    /// Enable a schedule trigger
    Enable { trigger: String },
    /// Disable a schedule trigger
    Disable { trigger: String },
    /// Fire a schedule trigger immediately
    RunNow { trigger: String },
}

pub async fn run(
    command: TriggersCommand,
    agent_id: String,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let api = ApiClient::new(api_url, api_key, org_id);
    let collection = format!("/v1/agents/{agent_id}/triggers");
    match command {
        TriggersCommand::List => {
            let value = api.get(&collection).await?;
            print_trigger_list(&value, output)
        }
        TriggersCommand::Create {
            cron_expression,
            timezone,
            session_mode,
            message,
            disabled,
        } => {
            let body = create_body(cron_expression, timezone, session_mode, message, !disabled);
            let value = api.post(&collection, Some(&body)).await?;
            print_trigger_action("Created", &value, output, quiet)
        }
        TriggersCommand::Update {
            trigger,
            cron_expression,
            timezone,
            session_mode,
            message,
        } => {
            let body = update_body(cron_expression, timezone, session_mode, message)?;
            let value = api.patch(&format!("{collection}/{trigger}"), &body).await?;
            print_trigger_action("Updated", &value, output, quiet)
        }
        TriggersCommand::Enable { trigger } => {
            set_enabled(&api, &collection, &trigger, true, output, quiet).await
        }
        TriggersCommand::Disable { trigger } => {
            set_enabled(&api, &collection, &trigger, false, output, quiet).await
        }
        TriggersCommand::RunNow { trigger } => {
            let value = api
                .post(&format!("{collection}/{trigger}/trigger"), None)
                .await?;
            if output.is_text() {
                if quiet {
                    if let Some(session_id) = value.get("session_id").and_then(Value::as_str) {
                        println!("{session_id}");
                    }
                } else {
                    println!("Triggered: {trigger}");
                    for (label, key) in [("Session", "session_id"), ("Message", "message_id")] {
                        if let Some(field) = value.get(key).and_then(Value::as_str) {
                            print_field(label, field);
                        }
                    }
                }
            } else {
                output.print_value(&value);
            }
            Ok(())
        }
    }
}

fn create_body(
    cron_expression: String,
    timezone: String,
    session_mode: SessionMode,
    message: String,
    enabled: bool,
) -> Value {
    json!({
        "cron_expression": cron_expression,
        "timezone": timezone,
        "session_mode": session_mode.as_api_value(),
        "message": message,
        "enabled": enabled,
    })
}

fn update_body(
    cron_expression: Option<String>,
    timezone: Option<String>,
    session_mode: Option<SessionMode>,
    message: Option<String>,
) -> Result<Value> {
    let mut body = serde_json::Map::new();
    if let Some(value) = cron_expression {
        body.insert("cron_expression".into(), json!(value));
    }
    if let Some(value) = timezone {
        body.insert("timezone".into(), json!(value));
    }
    if let Some(value) = session_mode {
        body.insert("session_mode".into(), json!(value.as_api_value()));
    }
    if let Some(value) = message {
        body.insert("message".into(), json!(value));
    }
    if body.is_empty() {
        anyhow::bail!(
            "update requires at least one of --cron, --timezone, --session-mode, or --message"
        );
    }
    Ok(Value::Object(body))
}

async fn set_enabled(
    api: &ApiClient<'_>,
    collection: &str,
    trigger: &str,
    enabled: bool,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let value = api
        .patch(
            &format!("{collection}/{trigger}"),
            &json!({ "enabled": enabled }),
        )
        .await?;
    print_trigger_action(
        if enabled { "Enabled" } else { "Disabled" },
        &value,
        output,
        quiet,
    )
}

fn print_trigger_list(value: &Value, output: OutputFormat) -> Result<()> {
    if !output.is_text() {
        output.print_value(value);
        return Ok(());
    }
    let triggers = value.as_array().context("Expected trigger list response")?;
    if triggers.is_empty() {
        println!("No triggers found.");
        return Ok(());
    }
    print_table_header(&[("ID", 38), ("SCHEDULE", 48), ("MODE", 24), ("ENABLED", 7)]);
    for trigger in triggers {
        let id = string_field(trigger, "id");
        let config = trigger.get("config").unwrap_or(&Value::Null);
        let cron = string_field(config, "cron_expression");
        let timezone = string_field(config, "timezone");
        let schedule = human_cron(cron, timezone);
        let mode = string_field(config, "session_mode");
        let enabled = trigger
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .to_string();
        print_table_row(&[(id, 38), (&schedule, 48), (mode, 24), (&enabled, 7)]);
    }
    Ok(())
}

fn print_trigger_action(
    action: &str,
    value: &Value,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    if !output.is_text() {
        output.print_value(value);
        return Ok(());
    }
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .context("Trigger response did not include id")?;
    if quiet {
        println!("{id}");
        return Ok(());
    }
    println!("{action} trigger: {id}");
    let config = value.get("config").unwrap_or(&Value::Null);
    print_field(
        "Schedule",
        &human_cron(
            string_field(config, "cron_expression"),
            string_field(config, "timezone"),
        ),
    );
    Ok(())
}

fn string_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("-")
}

pub(crate) fn human_cron(expression: &str, timezone: &str) -> String {
    let fields: Vec<_> = expression.split_whitespace().collect();
    let five = match fields.as_slice() {
        [minute, hour, day, month, weekday] => Some((*minute, *hour, *day, *month, *weekday)),
        ["0", minute, hour, day, month, weekday, "*"] => {
            Some((*minute, *hour, *day, *month, *weekday))
        }
        _ => None,
    };
    let description = match five {
        Some(("*", "*", "*", "*", "*")) => "Every minute".to_string(),
        Some((minute, "*", "*", "*", "*")) if minute.parse::<u8>().is_ok() => {
            format!("Every hour at :{:0>2}", minute)
        }
        Some((minute, hour, "*", "*", "*"))
            if minute.parse::<u8>().is_ok() && hour.parse::<u8>().is_ok() =>
        {
            format!("Every day at {:0>2}:{:0>2}", hour, minute)
        }
        _ => format!("Cron {expression}"),
    };
    format!("{description} ({timezone})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_five_and_seven_field_cron() {
        assert_eq!(human_cron("30 * * * *", "UTC"), "Every hour at :30 (UTC)");
        assert_eq!(
            human_cron("0 30 * * * * *", "America/Chicago"),
            "Every hour at :30 (America/Chicago)"
        );
        assert_eq!(human_cron("0 9 * * *", "UTC"), "Every day at 09:00 (UTC)");
    }

    #[test]
    fn update_requires_a_change() {
        assert!(update_body(None, None, None, None).is_err());
    }

    #[test]
    fn create_serializes_api_shape() {
        let body = create_body(
            "0 9 * * *".into(),
            "UTC".into(),
            SessionMode::SessionPerInvocation,
            "Daily report".into(),
            true,
        );
        assert_eq!(body["session_mode"], "session_per_invocation");
        assert_eq!(body["enabled"], true);
    }
}
