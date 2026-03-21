// CLI status command — show current user and org

use anyhow::Result;

use crate::auth::{CredentialStore, credentials_path};

pub fn run(profile: &str) -> Result<()> {
    let store = CredentialStore::load().unwrap_or_default();

    let profile_name = if profile.is_empty() {
        if store.current_profile.is_empty() {
            "default"
        } else {
            &store.current_profile
        }
    } else {
        profile
    };

    if let Some(p) = store.profiles.get(profile_name) {
        println!("Profile: {}", profile_name);
        println!("API URL: {}", p.api_url);
        println!(
            "API Key: {}...{}",
            &p.api_key[..std::cmp::min(12, p.api_key.len())],
            if p.api_key.len() > 16 {
                &p.api_key[p.api_key.len() - 4..]
            } else {
                ""
            }
        );
        if let Some(email) = &p.user_email {
            println!(
                "User:    {} ({})",
                p.user_name.as_deref().unwrap_or(""),
                email
            );
        }
        if let Some(org_id) = &p.org_id {
            println!("Org:     {}", org_id);
        }
    } else {
        let path_hint = credentials_path()
            .map(|p| format!(" (looked in {})", p.display()))
            .unwrap_or_default();
        eprintln!(
            "Not logged in. Run `everruns login` to authenticate.{}",
            path_hint
        );
        std::process::exit(1);
    }

    Ok(())
}
