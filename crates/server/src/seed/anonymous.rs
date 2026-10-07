//! Anonymous-user seeding and AUTH_MODE transition cleanup.
//!
// Kept out of `seed.rs` so that oversized-file ratchet can keep shrinking
// the orchestration module while mode-transition security stays next to the
// anonymous identity it protects.

use super::{SeedAuthContext, SeedResult, seed_admin_user, seed_default_organization};
use crate::auth::config::AuthMode;
use crate::org_init;
use crate::records::{ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME};
use crate::storage::{StorageBackend, models::CreateUserRow};
use everruns_core::DEFAULT_ORG_ID;

/// Seed anonymous user for auth=none mode.
/// Uses ANONYMOUS_USER_ID so all code paths (org membership, API keys, etc.)
/// work without special-casing a nil/missing user.
pub(super) async fn seed_anonymous_user(
    db: &StorageBackend,
    harness_definitions: &[crate::records::BuiltInHarnessDefinition],
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
    harness_definitions: &[crate::records::BuiltInHarnessDefinition],
) -> anyhow::Result<SeedResult> {
    let mut result = seed_anonymous_user(db, harness_definitions).await?;
    if auth_ctx.mode != AuthMode::None {
        result.merge(revoke_anonymous_personal_access_tokens(db).await?);
    }
    Ok(result)
}

/// Initialize request identities and mode-transition cleanup in seed order.
///
/// Startup awaits this before serving. Full seeding also calls it so standalone
/// seed callers retain the same idempotent organization/user prerequisites.
pub(super) async fn seed_auth_prerequisites(
    db: &StorageBackend,
    auth_ctx: &SeedAuthContext,
    built_in_harnesses: &[crate::records::BuiltInHarnessDefinition],
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    // Seed default organization first (all other resources depend on it)
    let org_result = seed_default_organization(db).await?;
    tracing::debug!(
        created = org_result.created,
        updated = org_result.updated,
        unchanged = org_result.unchanged,
        "Default organization seeded"
    );
    result.merge(org_result);

    // Anonymous identity + revoke of none-mode PATs when auth is enabled
    // (EVE-1153 / TM-AUTH-032). See `seed::anonymous`.
    let anon_result = seed_anonymous_user_for_auth_mode(db, auth_ctx, built_in_harnesses).await?;
    tracing::debug!(
        created = anon_result.created,
        updated = anon_result.updated,
        unchanged = anon_result.unchanged,
        "Anonymous user seeded"
    );
    result.merge(anon_result);

    // Seed admin user when in admin mode (depends on default org)
    if auth_ctx.mode == AuthMode::Admin
        && let Some(admin_config) = &auth_ctx.admin
    {
        let admin_result = seed_admin_user(db, admin_config, built_in_harnesses).await?;
        tracing::debug!(
            created = admin_result.created,
            updated = admin_result.updated,
            unchanged = admin_result.unchanged,
            "Admin user seeded"
        );
        result.merge(admin_result);
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

    async fn prepare_without_background(db: Arc<StorageBackend>, auth: &AuthConfig) {
        let task = crate::seed::prepare_seed_task(
            db,
            auth,
            crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev),
            crate::platform::oss_built_in_harnesses(),
            None,
        )
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn prepare_seed_task_exposes_runtime_owner_before_background() {
        let db = Arc::new(StorageBackend::test_database());
        prepare_without_background(db.clone(), &AuthConfig::default()).await;

        let mut caller = everruns_core::Caller::internal(DEFAULT_ORG_ID);
        caller.is_internal = false;
        caller.user_id = Some(ANONYMOUS_USER_ID);
        let owner = crate::services::PrincipalService::new(db.clone())
            .default_runtime_owner_principal(&caller, None)
            .await
            .expect("runtime ownership must be ready before background seeding");
        assert_eq!(owner.kind, "virtual_user");
        assert_eq!(owner.resolved_user_id, Some(ANONYMOUS_USER_ID));
        assert_eq!(
            db.get_organization_member(DEFAULT_ORG_ID, ANONYMOUS_USER_ID)
                .await
                .unwrap()
                .unwrap()
                .role,
            "owner"
        );
        assert!(
            db.get_harness_by_name(DEFAULT_ORG_ID, "conversation")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.list_providers(DEFAULT_ORG_ID).await.unwrap().is_empty(),
            "provider catalog seeding must remain in the background"
        );
    }

    async fn assert_prepared_pat_policy(mode: AuthMode) {
        let db = Arc::new(StorageBackend::test_database());
        super::super::seed_default_organization(&db).await.unwrap();
        seed_anonymous_user(&db, &crate::platform::oss_built_in_harnesses())
            .await
            .unwrap();
        let generated = crate::auth::personal_access_token::generate_personal_access_token();
        db.create_personal_access_token(CreatePersonalAccessTokenRow {
            user_id: ANONYMOUS_USER_ID,
            name: "existing-none-mode-token".into(),
            token_hash: generated.token_hash,
            token_prefix: generated.token_prefix,
            scopes: vec!["*".into()],
            expires_at: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
        let auth = AuthConfig {
            admin: (mode == AuthMode::Admin).then(|| crate::auth::config::AdminConfig {
                email: "startup-admin@example.com".into(),
                password: "development-test-password".into(),
            }),
            mode: mode.clone(),
            ..AuthConfig::default()
        };

        prepare_without_background(db.clone(), &auth).await;

        let tokens = db
            .list_personal_access_tokens_for_user(ANONYMOUS_USER_ID)
            .await
            .unwrap();
        assert_eq!(
            tokens.len(),
            usize::from(mode == AuthMode::None),
            "anonymous PAT policy must apply before serving {mode:?} traffic"
        );
        if let Some(admin) = auth.admin {
            let user = db.get_user_by_email(&admin.email).await.unwrap().unwrap();
            assert_eq!(
                db.get_organization_member(DEFAULT_ORG_ID, user.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .role,
                "owner"
            );
        }
    }

    #[tokio::test]
    async fn prepare_seed_task_revokes_anonymous_pats_in_admin_mode() {
        assert_prepared_pat_policy(AuthMode::Admin).await;
    }

    #[tokio::test]
    async fn prepare_seed_task_revokes_anonymous_pats_in_full_mode() {
        assert_prepared_pat_policy(AuthMode::Full).await;
    }

    #[tokio::test]
    async fn prepare_seed_task_revokes_anonymous_pats_in_external_mode() {
        assert_prepared_pat_policy(AuthMode::External).await;
    }

    #[tokio::test]
    async fn prepare_seed_task_preserves_anonymous_pats_in_none_mode() {
        assert_prepared_pat_policy(AuthMode::None).await;
    }

    #[tokio::test]
    async fn prepare_seed_task_preserves_operator_harness_definitions() {
        use crate::records::{BuiltInHarnessDefinition, BuiltInHarnessRole};

        let db = Arc::new(StorageBackend::test_database());
        let task = crate::seed::prepare_seed_task(
            db.clone(),
            &AuthConfig::default(),
            crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev),
            vec![
                BuiltInHarnessDefinition::new("operator-default", "Custom", "d", "p")
                    .with_roles([BuiltInHarnessRole::Base, BuiltInHarnessRole::Default]),
            ],
            None,
        )
        .await
        .unwrap();
        task.abort();
        let _ = task.await;

        let custom = db
            .get_harness_by_name(DEFAULT_ORG_ID, "operator-default")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            db.get_organization_settings(DEFAULT_ORG_ID)
                .await
                .unwrap()
                .unwrap()
                .default_harness_id,
            Some(custom.id)
        );
        assert!(
            db.get_harness_by_name(DEFAULT_ORG_ID, "conversation")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn prepare_seed_task_refuses_untrusted_admin_account() {
        let db = Arc::new(StorageBackend::test_database());
        let user = db
            .create_user(CreateUserRow {
                email: "startup-admin@example.com".into(),
                name: "Existing account".into(),
                avatar_url: None,
                roles: vec!["user".into()],
                password_hash: None,
                email_verified: false,
                auth_provider: Some("local".into()),
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();
        let auth = AuthConfig {
            mode: AuthMode::Admin,
            admin: Some(crate::auth::config::AdminConfig {
                email: user.email.clone(),
                password: "development-test-password".into(),
            }),
            ..AuthConfig::default()
        };
        match crate::seed::prepare_seed_task(
            db.clone(),
            &auth,
            crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev),
            crate::platform::oss_built_in_harnesses(),
            None,
        )
        .await
        {
            Err(error) => assert!(error.to_string().contains("Refusing to seed admin user")),
            Ok(task) => {
                task.abort();
                panic!("untrusted admin identity must stop startup");
            }
        }
        assert!(
            db.get_organization_member(DEFAULT_ORG_ID, user.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(db.list_providers(DEFAULT_ORG_ID).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn anonymous_pats_revoked_when_leaving_auth_mode_none() {
        let db = Arc::new(StorageBackend::test_database());

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
