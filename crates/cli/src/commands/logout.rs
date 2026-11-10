// CLI logout command — remove stored credentials

use anyhow::Result;

use crate::auth::CredentialStore;

pub fn run(profile: &str) -> Result<()> {
    let mut store = CredentialStore::load().unwrap_or_default();

    if store.remove_profile(profile) {
        store.save()?;
        crate::ui::ok(format!(
            "Logged out {}",
            crate::ui::dim(format!("(profile {profile})"))
        ));
    } else {
        crate::ui::note(format!(
            "Not logged in {}",
            crate::ui::dim(format!("(profile {profile})"))
        ));
    }

    Ok(())
}
