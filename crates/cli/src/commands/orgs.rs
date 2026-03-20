// CLI orgs commands — list and select organizations

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::auth::{CredentialStore, resolve_credentials};
use crate::output::OutputFormat;

#[derive(Deserialize)]
struct OrgInfo {
    public_id: String,
    name: String,
    role: String,
}

#[derive(Deserialize)]
struct MeResponse {
    organizations: Option<Vec<OrgInfo>>,
}

/// List organizations
pub async fn run_list(output_format: OutputFormat, profile: &str) -> Result<()> {
    let creds = match resolve_credentials(None, None, Some(profile)) {
        Ok(c) => c,
        Err(_) => {
            eprintln!(
                "Not logged in. Run `everruns login` or set EVERRUNS_API_KEY environment variable."
            );
            std::process::exit(1);
        }
    };
    let orgs = fetch_orgs(&creds.api_url, &creds.api_key).await?;

    match output_format {
        OutputFormat::Json => {
            let items: Vec<serde_json::Value> = orgs
                .iter()
                .map(|o| {
                    serde_json::json!({
                        "public_id": o.public_id,
                        "name": o.name,
                        "role": o.role,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        _ => {
            if orgs.is_empty() {
                println!("No organizations found.");
            } else {
                // Load current org for marking
                let store = CredentialStore::load().unwrap_or_default();
                let current_org = store.current_profile().and_then(|p| p.org_id.as_deref());

                for org in &orgs {
                    let marker = if current_org == Some(&org.public_id) {
                        " *"
                    } else {
                        ""
                    };
                    println!("  {} ({}){}", org.name, org.role, marker);
                }
            }
        }
    }

    Ok(())
}

/// Interactive org selection
pub async fn run_select(profile: &str) -> Result<()> {
    let creds = match resolve_credentials(None, None, Some(profile)) {
        Ok(c) => c,
        Err(_) => {
            eprintln!(
                "Not logged in. Run `everruns login` or set EVERRUNS_API_KEY environment variable."
            );
            std::process::exit(1);
        }
    };
    let orgs = fetch_orgs(&creds.api_url, &creds.api_key).await?;

    if orgs.is_empty() {
        return Err(anyhow!("No organizations found"));
    }

    if orgs.len() == 1 {
        eprintln!("Only one organization available: {}", orgs[0].name);
        return Ok(());
    }

    let items: Vec<String> = orgs
        .iter()
        .map(|o| format!("{} ({})", o.name, o.role))
        .collect();

    let selection = dialoguer::Select::new()
        .with_prompt("Select organization")
        .items(&items)
        .default(0)
        .interact()
        .context("Failed to select organization")?;

    let mut store = CredentialStore::load().unwrap_or_default();
    store.set_org(&orgs[selection].public_id)?;
    store.save()?;

    eprintln!("Switched to organization: {}", orgs[selection].name);

    Ok(())
}

/// Fetch orgs from /v1/auth/me
async fn fetch_orgs(api_url: &str, api_key: &str) -> Result<Vec<OrgInfo>> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/v1/auth/me", api_url))
        .header("Authorization", api_key)
        .send()
        .await
        .context("Failed to connect to Everruns server")?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "Failed to fetch user info ({}). Is your API key valid?",
            resp.status()
        ));
    }

    let me: MeResponse = resp.json().await.context("Failed to parse response")?;
    Ok(me.organizations.unwrap_or_default())
}
