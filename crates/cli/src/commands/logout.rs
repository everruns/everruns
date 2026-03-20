// CLI logout command — remove stored credentials

use anyhow::Result;

use crate::auth::CredentialStore;

pub fn run(profile: &str) -> Result<()> {
    let mut store = CredentialStore::load().unwrap_or_default();

    if store.remove_profile(profile) {
        store.save()?;
        eprintln!("Logged out (profile: {})", profile);
    } else {
        eprintln!("Not logged in (profile: {})", profile);
    }

    Ok(())
}
