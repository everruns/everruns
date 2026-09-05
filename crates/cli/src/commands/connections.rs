// Connection management commands
//
// `remove` uses the SDK's `client.connections().remove()`.
// `set` and `list` still use raw reqwest because the SDK's `Connection`
// model lacks `connection_type`, `provider_username`, `scopes`, and uses
// `created_at`/`updated_at` instead of the server's `connected_at`.
// The SDK's `list()` also expects `ListResponse<Connection>` but the
// server returns a plain `Vec<ConnectionResponse>`.
// TODO(everruns/sdk#66): Fix SDK Connection model to match server, then migrate set/list

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use dialoguer::Password;
use everruns_sdk::Everruns;
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Subcommand)]
pub enum ConnectionsCommand {
    /// Set an API key connection for a provider
    Set {
        /// Provider name (e.g. daytona, brave_search, browserless, deno, sprites)
        provider: String,

        /// Read provider API key from stdin
        #[arg(long)]
        api_key_stdin: bool,
    },

    /// List connected providers
    List,

    /// Remove a connection
    Remove {
        /// Provider name (e.g. daytona, brave_search)
        provider: String,
    },
}

#[derive(Debug, Deserialize)]
struct ConnectionResponse {
    provider: String,
    connection_type: String,
    provider_username: Option<String>,
    connected_at: String,
}

#[derive(Debug, Serialize)]
struct CreateApiKeyRequest {
    api_key: String,
}

pub async fn run(
    command: ConnectionsCommand,
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        ConnectionsCommand::Set {
            provider,
            api_key_stdin,
        } => {
            let provider_api_key = read_provider_api_key(api_key_stdin)?;
            set(
                api_url,
                api_key,
                output,
                quiet,
                &provider,
                &provider_api_key,
            )
            .await
        }
        ConnectionsCommand::List => list(api_url, api_key, output).await,
        ConnectionsCommand::Remove { provider } => remove(client, output, quiet, &provider).await,
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Build a connection API URL, normalizing trailing slashes.
fn connection_url(api_url: &str, path: &str) -> String {
    format!("{}{}", api_url.trim_end_matches('/'), path)
}

fn read_provider_api_key(api_key_stdin: bool) -> Result<String> {
    if api_key_stdin {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .context("Failed to read provider API key from stdin")?;
        let api_key = input.trim().to_string();
        if api_key.is_empty() {
            anyhow::bail!("Provider API key from stdin cannot be empty");
        }
        return Ok(api_key);
    }

    Password::new()
        .with_prompt("Provider API key")
        .allow_empty_password(false)
        .interact()
        .context("Failed to read provider API key. Use --api-key-stdin for non-interactive usage")
}

async fn set(
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
    provider: &str,
    provider_api_key: &str,
) -> Result<()> {
    let resp = http_client()
        .post(connection_url(
            api_url,
            &format!("/v1/user/connections/{}", provider),
        ))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&CreateApiKeyRequest {
            api_key: provider_api_key.to_string(),
        })
        .send()
        .await
        .context("Failed to connect to server")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Failed to set connection for {}: {} {}",
            provider,
            status,
            body
        );
    }

    let conn: ConnectionResponse = resp
        .json()
        .await
        .context("Failed to parse connection response")?;

    if output.is_text() {
        if quiet {
            println!("{}", conn.provider);
        } else {
            println!("Connected: {}", conn.provider);
            if let Some(username) = &conn.provider_username {
                print_field("Username", username);
            }
            print_field("Type", &conn.connection_type);
        }
    } else {
        output.print_value(&serde_json::json!({
            "provider": conn.provider,
            "connection_type": conn.connection_type,
            "provider_username": conn.provider_username,
            "connected_at": conn.connected_at,
        }));
    }

    Ok(())
}

async fn list(api_url: &str, api_key: &str, output: OutputFormat) -> Result<()> {
    let resp = http_client()
        .get(connection_url(api_url, "/v1/user/connections"))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .context("Failed to connect to server")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Failed to list connections: {} {}", status, body);
    }

    let connections: Vec<ConnectionResponse> = resp
        .json()
        .await
        .context("Failed to parse connections response")?;

    if output.is_text() {
        if connections.is_empty() {
            println!("No connections found");
            return Ok(());
        }

        print_table_header(&[
            ("PROVIDER", 20),
            ("TYPE", 10),
            ("USERNAME", 20),
            ("CONNECTED", 20),
        ]);

        for conn in &connections {
            let username = conn.provider_username.as_deref().unwrap_or("-");
            print_table_row(&[
                (&conn.provider, 20),
                (&conn.connection_type, 10),
                (username, 20),
                (&conn.connected_at, 20),
            ]);
        }
    } else {
        output.print_value(&serde_json::json!({
            "data": connections.iter().map(|c| serde_json::json!({
                "provider": c.provider,
                "connection_type": c.connection_type,
                "provider_username": c.provider_username,
                "connected_at": c.connected_at,
            })).collect::<Vec<_>>()
        }));
    }

    Ok(())
}

async fn remove(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    provider: &str,
) -> Result<()> {
    if let Err(e) = client.connections().remove(provider).await {
        match &e {
            everruns_sdk::Error::Api { status: 404, .. } => {
                anyhow::bail!("Connection not found: {}", provider);
            }
            _ => {
                return Err(e)
                    .with_context(|| format!("Failed to remove connection for {}", provider));
            }
        }
    }

    if output.is_text() {
        if !quiet {
            println!("Disconnected: {}", provider);
        }
    } else {
        output.print_value(&serde_json::json!({
            "provider": provider,
            "status": "disconnected",
        }));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_api_key_request_serialization() {
        let req = CreateApiKeyRequest {
            api_key: "test_key_123".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            serde_json::json!({"api_key": "test_key_123"})
        );
    }

    #[test]
    fn test_connection_response_deserialization() {
        let json = r#"{
            "provider": "daytona",
            "connection_type": "api_key",
            "provider_username": "user@example.com",
            "connected_at": "2024-01-01T00:00:00Z"
        }"#;
        let conn: ConnectionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(conn.provider, "daytona");
        assert_eq!(conn.connection_type, "api_key");
        assert_eq!(conn.provider_username.as_deref(), Some("user@example.com"));
        assert_eq!(conn.connected_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn test_connection_response_deserialization_no_username() {
        let json = r#"{
            "provider": "brave_search",
            "connection_type": "api_key",
            "provider_username": null,
            "connected_at": "2024-01-01T00:00:00Z"
        }"#;
        let conn: ConnectionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(conn.provider, "brave_search");
        assert!(conn.provider_username.is_none());
    }
}
