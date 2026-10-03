//! Anonymous-user seeding and AUTH_MODE transition cleanup.
//!
//! Kept out of `seed.rs` so that oversized-file ratchet can keep shrinking
//! the orchestration module while mode-transition security stays next to the
//! anonymous identity it protects.

use super::{SeedAuthContext, SeedResult};
use crate::auth::config::AuthMode;
use crate::org_init;
use crate::storage::{StorageBackend, models::CreateUserRow};
use everruns_core::DEFAULT_ORG_ID;
use everruns_platform::{ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME};

/// Seed anonymous user for auth=none mode.
/// Uses ANONYMOUS_USER_ID so all code paths (org membership, API keys, etc.)
/// work without special-casing a nil/missing user.
pub(super) async fn seed_anonymous_user(
    db: &StorageBackend,
    harness_definitions: &[everruns_platform::BuiltInHarnessDefinition],
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    let input = CreateUserRow {
        email: ANONYMOUS_USER_EMAIL.to_string(),
        name: ANONYMOUS_USER_NAME.to_string(),
        avatar_url: None,
        roles: vec!["admin".to_string()],
        password_hash: None,
        email_verified: true,
        auth_provider: Some("none".to_string()),
        auth_provider_id: None,
        external_id: None,
    };

    match db.create_user_with_id(ANONYMOUS_USER_ID, input).await? {
        Some(_) => {
            tracing::info!("Created anonymous user");
            result.created += 1;
        }
        None => {
            tracing::debug!("Anonymous user up to date");
            result.unchanged += 1;
        }
    }

    // Ensure anonymous user is owner of default org
    db.ensure_membership(ANONYMOUS_USER_ID, DEFAULT_ORG_ID, "owner")
        .await?;

    // Ensure default org has built-in harnesses (same safety net as registration handlers)
    org_init::initialize_org_harnesses_with_definitions(db, DEFAULT_ORG_ID, harness_definitions)
        .await?;

    Ok(result)
}

/// Revoke outstanding anonymous-user PATs when authentication is enabled.
///
/// The anonymous admin identity is always seeded so none-mode code paths stay
/// uniform, but a PAT minted under `AUTH_MODE=none` must not remain valid after
/// the same database is restarted in admin/full/external mode (EVE-1153).
///
/// THREAT[TM-AUTH-032]: mode transition must invalidate persisted anonymous
/// credentials, not only change the live request middleware.
pub(super) async fn revoke_anonymous_personal_access_tokens(
    db: &StorageBackend,
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();
    let revoked = db
        .delete_personal_access_tokens_for_user(ANONYMOUS_USER_ID)
        .await?;
    if revoked > 0 {
        tracing::warn!(
            revoked,
            user_id = %ANONYMOUS_USER_ID,
            "Revoked anonymous-user personal access tokens after leaving AUTH_MODE=none"
        );
        result.updated += revoked as usize;
    } else {
        result.unchanged += 1;
    }
    Ok(result)
}

/// Seed the anonymous identity, then revoke its PATs when leaving none mode.
pub(super) async fn seed_anonymous_user_for_auth_mode(
    db: &StorageBackend,
    auth_ctx: &SeedAuthContext,
    harness_definitions: &[everruns_platform::BuiltInHarnessDefinition],
) -> anyhow::Result<SeedResult> {
    let mut result = seed_anonymous_user(db, harness_definitions).await?;
    if auth_ctx.mode != AuthMode::None {
        result.merge(revoke_anonymous_personal_access_tokens(db).await?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::backend::AuthBackend;
    use crate::auth::builtin::BuiltinAuthBackend;
    use crate::auth::config::AuthConfig;
    use crate::seed::seed_all;
    use crate::storage::models::CreatePersonalAccessTokenRow;
    use everruns_core::DeploymentGrade;
    use std::sync::Arc;

    #[tokio::test]
    async fn anonymous_pats_revoked_when_leaving_auth_mode_none() {
        let db = Arc::new(StorageBackend::in_memory());

        // Mint under AUTH_MODE=none against a persisted database.
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        let generated = crate::auth::personal_access_token::generate_personal_access_token();
        db.create_personal_access_token(CreatePersonalAccessTokenRow {
            user_id: ANONYMOUS_USER_ID,
            name: "dev-token".to_string(),
            token_hash: generated.token_hash.clone(),
            token_prefix: generated.token_prefix.clone(),
            scopes: vec!["*".to_string()],
            expires_at: None,
            metadata: serde_json::json!({"source": "test"}),
        })
        .await
        .unwrap();

        let before = db
            .list_personal_access_tokens_for_user(ANONYMOUS_USER_ID)
            .await
            .unwrap();
        assert_eq!(
            before.len(),
            1,
            "none-mode PAT should persist before transition"
        );

        // Restart the same database in an authenticated mode.
        seed_all(
            &db,
            DeploymentGrade::Dev,
            &SeedAuthContext {
                mode: AuthMode::Full,
                admin: None,
            },
        )
        .await
        .unwrap();

        let after = db
            .list_personal_access_tokens_for_user(ANONYMOUS_USER_ID)
            .await
            .unwrap();
        assert!(
            after.is_empty(),
            "anonymous PATs must be revoked when leaving AUTH_MODE=none"
        );

        let full_backend = BuiltinAuthBackend::new(
            AuthConfig {
                mode: AuthMode::Full,
                ..AuthConfig::default()
            },
            db.clone(),
            Arc::new(crate::platform::oss_host_composition()),
        );
        let rejected = full_backend
            .validate_personal_access_token(&generated.token)
            .await;
        assert!(
            rejected.is_err(),
            "old anonymous PAT must not authenticate after enabling auth"
        );

        // Intentional none-mode behavior remains: re-entering none keeps the
        // anonymous identity and allows minting a fresh PAT for local use.
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        let fresh = crate::auth::personal_access_token::generate_personal_access_token();
        db.create_personal_access_token(CreatePersonalAccessTokenRow {
            user_id: ANONYMOUS_USER_ID,
            name: "fresh-none-token".to_string(),
            token_hash: fresh.token_hash,
            token_prefix: fresh.token_prefix,
            scopes: vec!["*".to_string()],
            expires_at: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
        let none_tokens = db
            .list_personal_access_tokens_for_user(ANONYMOUS_USER_ID)
            .await
            .unwrap();
        assert_eq!(
            none_tokens.len(),
            1,
            "none mode must still allow anonymous PAT minting"
        );
    }
}
