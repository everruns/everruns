//! Local authenticated capacity fixture for `just bench-chat-capacity`.
//! A utility binary keeps its dependencies identical to production builds.
//!
//! Uses production HTTP/gRPC composition and a standalone worker. Only the local
//! IdP's public discovery/key cache is preloaded: the normal SSRF guard correctly
//! rejects loopback IdPs. Signature, audience, expiry, binding, channel and DB
//! authorization still run. There is no production bypass or test HTTP endpoint.
//! The coordinator strips provider credentials and uses a disposable database.

use std::time::Duration;

use anyhow::Result;
use everruns_core::channel_auth::ChannelAuthVerifier;
use everruns_server::{app_builder::ServerAppBuilder, server::ServerConfig};

// Match the production server binary's allocator under concurrent allocations.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("CAPACITY_ISOLATED").as_deref() != Ok("1") {
        anyhow::bail!("capacity fixture requires its isolated local coordinator");
    }
    if std::env::var("AUTH_MODE").as_deref() != Ok("full") {
        anyhow::bail!("capacity fixture requires full authentication");
    }
    if std::env::vars().any(|(key, value)| {
        !value.is_empty() && (key.ends_with("API_KEY") || key.starts_with("UTILITY_"))
    }) {
        anyhow::bail!("capacity fixture must start without provider credentials");
    }
    let database = url::Url::parse(&std::env::var("DATABASE_URL")?)?;
    if !matches!(database.host_str(), Some("127.0.0.1" | "localhost"))
        || !database.path().starts_with("/capacity_")
    {
        anyhow::bail!("capacity fixture requires a disposable local capacity_ database");
    }
    let path = std::env::var("CAPACITY_IDP_PUBLIC")?;
    let document: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let issuer = document["issuer"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("fixture issuer missing"))?
        .to_owned();
    let jwks_uri = format!("{issuer}/jwks");
    let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_value(document["jwks"].clone())?;
    let verifier = ChannelAuthVerifier::new();
    verifier.prime_oidc(&issuer, &jwks_uri, jwks.clone()).await;
    let warm = verifier.clone();
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(300));
        loop {
            timer.tick().await;
            warm.prime_oidc(&issuer, &jwks_uri, jwks.clone()).await;
        }
    });
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }
    let credentials = std::env::var("CAPACITY_USERS_PRIVATE")?;
    let users: usize = std::env::var("CAPACITY_USERS")?.parse()?;
    if users == 0 || users > 10_000 {
        anyhow::bail!("capacity fixture user count outside bounds");
    }
    ServerAppBuilder::new(ServerConfig::from_env())
        .channel_auth_verifier(verifier)
        .background_task(move |context| async move {
            if let Err(error) = seed_users(context, &credentials, users).await {
                tracing::error!(error = %error, "capacity identity fixture failed");
            }
        })
        .run()
        .await
}

async fn seed_users(
    context: everruns_server::app_builder::ServerContext,
    path: &str,
    count: usize,
) -> Result<()> {
    use everruns_server::auth::{
        AuthConfig,
        jwt::{JwtService, hash_token},
    };
    use everruns_server::storage::{CreateRefreshTokenRow, CreateUserRow};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if std::path::Path::new(path).exists() {
        return Ok(()); // server restart preserves the same admitted test users
    }
    let config = AuthConfig::from_env();
    let service = JwtService::new(config.jwt.clone());
    let admin_email = "capacity-admin@bench.invalid";
    let admin = match context.db.get_user_by_email(admin_email).await? {
        Some(user) => user,
        None => {
            context
                .db
                .create_user(CreateUserRow {
                    email: admin_email.into(),
                    name: "Capacity control".into(),
                    avatar_url: None,
                    roles: vec!["admin".into()],
                    password_hash: None,
                    email_verified: true,
                    auth_provider: Some("capacity-fixture".into()),
                    auth_provider_id: None,
                    external_id: None,
                })
                .await?
        }
    };
    let admin_token =
        service.generate_access_token(admin.id, &admin.email, &admin.name, &["admin".into()])?;
    let mut users = Vec::new();
    for index in 0..count {
        let user = context
            .db
            .create_user(CreateUserRow {
                email: format!("capacity-{index}@bench.invalid"),
                name: format!("Capacity user {index}"),
                avatar_url: None,
                roles: Vec::new(),
                password_hash: None,
                email_verified: true,
                auth_provider: Some("capacity-fixture".into()),
                auth_provider_id: None,
                external_id: None,
            })
            .await?;
        let (tokens, _) = service.generate_token_pair(user.id, &user.email, &user.name, &[])?;
        context
            .db
            .create_refresh_token(CreateRefreshTokenRow {
                user_id: user.id,
                token_hash: hash_token(&tokens.refresh_token),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(4),
            })
            .await?;
        users.push(serde_json::json!({"id": user.id, "cookie": format!("access_token={}; refresh_token={}", tokens.access_token, tokens.refresh_token)}));
    }
    let temporary = format!("{path}.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(
        &serde_json::json!({"admin": {"access_token":admin_token}, "users": users}),
    )?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
