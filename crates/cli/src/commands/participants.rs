use crate::commands::api::ApiClient;
use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};

#[derive(Subcommand, Debug)]
pub enum ParticipantsCommand {
    /// List current and past session participants
    List,
    /// Add (invite) an agent as a member
    #[command(alias = "invite")]
    Add {
        /// Agent ID to invite
        #[arg(long)]
        agent: String,
    },
    /// Remove a participant from the session
    Remove {
        /// Participant ID (e.g. part_xxx)
        participant: String,
    },
}

pub async fn run(
    command: ParticipantsCommand,
    session_id: String,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let api = ApiClient::new(api_url, api_key, org_id);
    let collection = format!("/v1/sessions/{session_id}/participants");
    match command {
        ParticipantsCommand::List => {
            let value = api.get(&collection).await?;
            print_participant_list(&value, output)
        }
        ParticipantsCommand::Add { agent } => {
            let value = api
                .post(
                    &collection,
                    Some(&json!({ "kind": "agent", "agent_id": agent })),
                )
                .await?;
            print_participant_action("Added", &value, output, quiet)
        }
        ParticipantsCommand::Remove { participant } => {
            let value = api.delete(&format!("{collection}/{participant}")).await?;
            print_participant_action("Removed", &value, output, quiet)
        }
    }
}

fn print_participant_list(value: &Value, output: OutputFormat) -> Result<()> {
    if !output.is_text() {
        output.print_value(value);
        return Ok(());
    }
    let participants = value
        .as_array()
        .context("Expected participant list response")?;
    if participants.is_empty() {
        println!("No participants found.");
        return Ok(());
    }
    print_table_header(&[
        ("ID", 38),
        ("KIND", 8),
        ("ROLE", 8),
        ("AGENT", 38),
        ("STATUS", 8),
    ]);
    for participant in participants {
        let status = if participant.get("left_at").is_some_and(|v| !v.is_null()) {
            "left"
        } else {
            "active"
        };
        print_table_row(&[
            (string_field(participant, "id"), 38),
            (string_field(participant, "kind"), 8),
            (string_field(participant, "role"), 8),
            (string_field(participant, "agent_id"), 38),
            (status, 8),
        ]);
    }
    Ok(())
}

fn print_participant_action(
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
        .context("Participant response did not include id")?;
    if quiet {
        println!("{id}");
    } else {
        println!("{action} participant: {id}");
        print_field("Role", string_field(value, "role"));
        if let Some(agent_id) = value.get("agent_id").and_then(Value::as_str) {
            print_field("Agent", agent_id);
        }
    }
    Ok(())
}

fn string_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("-")
}
